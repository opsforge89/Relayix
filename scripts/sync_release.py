#!/usr/bin/env python3
"""Set Cargo.toml's package version, then run this to refresh release manifests."""
from pathlib import Path
import json
import re
import sys
import tomllib

root=Path(__file__).resolve().parents[1]
version=tomllib.loads((root/'Cargo.toml').read_text())['package']['version']
files={
 'docs/GITHUB.md':r'(?<=Relayix )\d+\.\d+\.\d+|(?<=v)\d+\.\d+\.\d+|(?<=relayix:)\d+\.\d+\.\d+|(?<=`)\d+\.\d+\.\d+(?=`)',
 'docs/DEPLOYMENT_ADVANCED.md':r'(?<=relayix:)\d+\.\d+\.\d+',
 '.github/workflows/deploy.yml':r'(?<=for example )\d+\.\d+\.\d+',
 '.github/workflows/release.yml':r'(?<=for example )\d+\.\d+\.\d+',

 'compose.yaml':r'(?<=relayix:)\d+\.\d+\.\d+',
 'README.md':r'(?<=relayix:)\d+\.\d+\.\d+',
 'scripts/check.sh':r'(?<=:)\d+\.\d+\.\d+',
 'tests/compose.yaml':r'(relayix(?:-checks|-browser)?:)\d+\.\d+\.\d+',
 'tests/compose_databases.py':r'(?<=relayix:)\d+\.\d+\.\d+',
 'tests/compose_modes.py':r'(?<=relayix:)\d+\.\d+\.\d+',
 'tests/startup.py':r'(?<=relayix:)\d+\.\d+\.\d+',
 'tests/upgrade.py':r"(?<=RELAYIX_TEST_IMAGE','relayix:)\d+\.\d+\.\d+|(?<='version']==')\d+\.\d+\.\d+",
}
changed=[]
for name,pattern in files.items():
 path=root/name;text=path.read_text();updated=re.sub(pattern,lambda match:(match.group(1) if match.lastindex else '')+version,text)
 if updated!=text:
  changed.append(name)
  if '--check' not in sys.argv:path.write_text(updated)
for name in ['web/package.json','web/package-lock.json']:
 path=root/name;text=path.read_text();data=json.loads(text);data['version']=version
 if 'packages' in data:data['packages']['']['version']=version
 updated=json.dumps(data,indent=2)+'\n'
 if updated!=text:
  changed.append(name)
  if '--check' not in sys.argv:path.write_text(updated)
path=root/'Cargo.lock';text=path.read_text();updated=re.sub(r'(name = "relayix"\nversion = ")[^"]+',r'\g<1>'+version,text)
if updated!=text:
 changed.append('Cargo.lock')
 if '--check' not in sys.argv:path.write_text(updated)
if changed and '--check' in sys.argv:raise SystemExit('Run python3 scripts/sync_release.py: '+', '.join(changed))
print('Release manifests match Cargo version '+version)
