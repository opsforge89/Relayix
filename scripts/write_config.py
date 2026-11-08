#!/usr/bin/env python3
"""Render a private candidate config from GitHub Secrets/Variables or fresh values."""
import argparse
import json
import os
from pathlib import Path
import secrets

ROOT=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('output');parser.add_argument('--generate',action='store_true');parser.add_argument('--base');parser.add_argument('--database',choices=['local','external']);args=parser.parse_args()
path=Path(args.output).resolve()
if path.is_relative_to(ROOT):raise SystemExit('Configuration output must be outside the source checkout')
if path.exists():raise SystemExit('Refusing to overwrite existing configuration')
config=json.loads((ROOT/'config'/'deployment.example.json').read_text())
if args.base:
    base=Path(args.base).resolve()
    if base.is_relative_to(ROOT):raise SystemExit('Base config must be outside checkout')
    config.update(json.loads(base.read_text()))
# The app image is selected in compose.yaml, not in private settings.
config.pop('RELAYIX_IMAGE',None)
keys=set(config)|{'POSTGRES_PASSWORD','ADMIN_TOKEN','ADMIN_PASSWORD','TRUSTED_PROXIES'}
for key in keys:
    if os.environ.get(key):config[key]=os.environ[key]
if args.database:config['DATABASE_MODE']=args.database
mode=config.get('DATABASE_MODE','local')
if mode not in ('local','external'):raise SystemExit('DATABASE_MODE must be local or external')
if args.generate:
    if mode=='local' and not config.get('POSTGRES_PASSWORD'):config['POSTGRES_PASSWORD']=secrets.token_hex(32)
    if mode=='external':config.setdefault('POSTGRES_PASSWORD','')
    if not config.get('ADMIN_TOKEN'):config['ADMIN_TOKEN']=secrets.token_urlsafe(48)
    if not config.get('ADMIN_PASSWORD'):
        while True:
            candidate='A7!b'+secrets.token_hex(24)
            if '123456' not in candidate:break
        config['ADMIN_PASSWORD']=candidate
if not config.get('POSTGRES_PASSWORD') and not (args.generate and mode=='external'):raise SystemExit('POSTGRES_PASSWORD is required; external databases use their existing password')
if not isinstance(config,dict) or any(not isinstance(k,str) or not isinstance(v,str) for k,v in config.items()):raise SystemExit('Deployment configuration must contain string values')
if any('\n' in v or '\r' in v or '\x00' in v for v in config.values()):raise SystemExit('Multiline configuration values are not allowed')
os.umask(0o077);path.parent.mkdir(parents=True,exist_ok=True);path.parent.chmod(0o700)
path.write_text(json.dumps(config,indent=2)+'\n');path.chmod(0o600)
print('Private configuration written; secret values omitted.')
if mode=='external':print('Before starting, set POSTGRES_HOST, POSTGRES_USER, POSTGRES_DB and the existing POSTGRES_PASSWORD in the private JSON.')
