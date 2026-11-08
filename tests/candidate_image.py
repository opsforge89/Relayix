"""Ensure the promoted digest's runtime is exactly the runtime used by CI."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
def inspect(name):
    return json.loads(subprocess.check_output(['docker', 'image', 'inspect', name], text=True))[0]

production = inspect('relayix:ci')
verified = inspect('relayix:verified')
production_layers = production['RootFS']['Layers']
verified_layers = verified['RootFS']['Layers']
assert verified_layers[:len(production_layers)] == production_layers, 'Verification runtime differs from the candidate'
assert len(verified_layers) == len(production_layers) + 2, 'Unexpected verification-only layers'
for field in ['Env', 'Cmd', 'Entrypoint', 'WorkingDir', 'User', 'Healthcheck', 'ExposedPorts']:
    assert production['Config'].get(field) == verified['Config'].get(field), field
for image in [production, verified]:
    labels = image['Config']['Labels']
    assert labels['org.opencontainers.image.revision'] == os.environ['REVISION']
    assert labels['org.opencontainers.image.version'] == os.environ['VERSION']

with tempfile.TemporaryDirectory(prefix='relayix-candidate-metadata-') as folder:
    root = Path(folder)
    metadata = root / 'metadata.json'
    output = root / 'outputs'
    env = {**os.environ, 'GITHUB_OUTPUT': str(output), 'CANDIDATE_REF_FILE': str(root / 'candidate-ref')}
    digest = 'sha256:' + 'f' * 64
    metadata.write_text(json.dumps({'production': {'containerimage.digest': digest}, 'verified': {'containerimage.digest': 'sha256:' + '0' * 64}}))
    subprocess.run([sys.executable, str(ROOT / 'scripts/pin_candidate.py'), str(metadata)], env=env, check=True, capture_output=True)
    assert 'image_ref=' + env['IMAGE'] + '@' + digest in output.read_text()
    for invalid in [{'verified': {'containerimage.digest': digest}}, {'production': {'containerimage.digest': 'not-a-digest'}}]:
        metadata.write_text(json.dumps(invalid))
        before = output.read_text()
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/pin_candidate.py'), str(metadata)], env=env, capture_output=True)
        assert result.returncode != 0 and output.read_text() == before
print('PASS: exact production runtime, verification layers, revision/version labels and digest-only promotion metadata')
