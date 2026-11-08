#!/usr/bin/env python3
"""Pin the production export, including attestations, before any runtime tests."""
import json
import os
from pathlib import Path
import re
import sys

metadata = json.loads(Path(sys.argv[1]).read_text())
digest = metadata['production']['containerimage.digest']
if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
    raise SystemExit('Invalid production image digest')
image = os.environ['IMAGE']
reference = image + '@' + digest
with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
    output.write('image_ref=' + reference + '\n')
    output.write('image=' + image + '\n')
    output.write('version=' + os.environ['VERSION'] + '\n')
Path(os.environ.get('CANDIDATE_REF_FILE', '/tmp/relayix-candidate-ref')).write_text(reference + '\n')
print('Runtime tests and release promotion use ' + reference)
