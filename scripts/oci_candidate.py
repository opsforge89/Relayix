#!/usr/bin/env python3
"""Select one OCI root without rewriting its manifest, layers or attestations."""
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile
import tempfile


def normalize(path, digest):
    path = Path(path)
    if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
        raise ValueError('Invalid candidate digest')
    blob_name = 'blobs/sha256/' + digest.removeprefix('sha256:')
    with tarfile.open(path, 'r') as source:
        members = {m.name.removeprefix('./'): m for m in source.getmembers()}
        root = members.get(blob_name, members.get('index.json'))
        if root is None or not root.isfile() or root.size > 16 * 1024 * 1024:
            raise ValueError('Missing or oversized OCI root')
        raw = source.extractfile(root).read()
        if 'sha256:' + hashlib.sha256(raw).hexdigest() != digest:
            raise ValueError('OCI archive root differs from the candidate digest')
        manifest = json.loads(raw)
        media = manifest.get('mediaType')
        if media not in ('application/vnd.oci.image.index.v1+json', 'application/vnd.oci.image.manifest.v1+json', 'application/vnd.docker.distribution.manifest.list.v2+json', 'application/vnd.docker.distribution.manifest.v2+json'):
            raise ValueError('Unsupported OCI root media type')
        descriptor = {'mediaType': media, 'digest': digest, 'size': len(raw), 'annotations': {'org.opencontainers.image.ref.name': 'candidate'}}
        wrapper = json.dumps({'schemaVersion': 2, 'manifests': [descriptor]}, separators=(',', ':')).encode()
        with tempfile.TemporaryDirectory(prefix='relayix-oci-', dir=path.parent) as directory:
            target = Path(directory) / 'candidate.tar'
            with tarfile.open(target, 'w') as output:
                for name, member in members.items():
                    if name == 'index.json':
                        continue
                    output.addfile(member, source.extractfile(member) if member.isfile() else None)
                if blob_name not in members:
                    info = tarfile.TarInfo(blob_name); info.size = len(raw); info.mode = 0o644
                    output.addfile(info, io.BytesIO(raw))
                info = tarfile.TarInfo('index.json'); info.size = len(wrapper); info.mode = 0o644
                output.addfile(info, io.BytesIO(wrapper))
            target.replace(path)
    return descriptor


if __name__ == '__main__':
    import sys
    metadata = json.loads(Path(sys.argv[2]).read_text())
    normalize(sys.argv[1], metadata['production']['containerimage.digest'])
    print('OCI archive selects the exact candidate root, including all attestations.')
