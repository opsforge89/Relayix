"""Archive packaging must preserve the exact root that will later be promoted."""
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from oci_candidate import normalize


class CandidateArchiveTests(unittest.TestCase):
    def test_flat_index_and_attestations_keep_their_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / 'candidate.tar'
            root = json.dumps({'schemaVersion': 2, 'mediaType': 'application/vnd.oci.image.index.v1+json', 'manifests': [{'digest': 'sha256:' + '1' * 64}, {'digest': 'sha256:' + '2' * 64, 'platform': {'os': 'unknown', 'architecture': 'unknown'}}]}).encode()
            digest = 'sha256:' + hashlib.sha256(root).hexdigest()
            with tarfile.open(archive, 'w') as output:
                for name, value in [('index.json', root), ('oci-layout', b'{"imageLayoutVersion":"1.0.0"}'), ('blobs/sha256/'+'1'*64, b'runtime'), ('blobs/sha256/'+'2'*64, b'attestation')]:
                    info = tarfile.TarInfo(name); info.size = len(value)
                    output.addfile(info, io.BytesIO(value))
            normalize(archive, digest)
            normalize(archive, digest)  # Reruns are safe for an already wrapped archive.
            with tarfile.open(archive) as source:
                wrapper = json.load(source.extractfile('index.json'))
                self.assertEqual(len(wrapper['manifests']), 1)
                self.assertEqual(wrapper['manifests'][0]['digest'], digest)
                self.assertEqual(wrapper['manifests'][0]['annotations']['org.opencontainers.image.ref.name'], 'candidate')
                self.assertEqual(source.extractfile('blobs/sha256/'+digest.split(':')[1]).read(), root)
                self.assertEqual(source.extractfile('blobs/sha256/'+'2'*64).read(), b'attestation')
            with self.assertRaises(ValueError):
                normalize(archive, 'sha256:' + 'f' * 64)


if __name__ == '__main__':
    unittest.main()
