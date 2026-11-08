"""Create per-run test credentials outside the source checkout; never print values."""
import json,os,secrets,string,sys,tomllib
from pathlib import Path

root=Path(__file__).resolve().parents[1];target=Path(sys.argv[1]).resolve()
if target.is_relative_to(root):raise SystemExit('Test runtime credentials must be outside the checkout')
os.umask(0o077);target.mkdir(parents=True,exist_ok=True);target.chmod(0o700)
def value():return secrets.token_urlsafe(36)
def password():
    while True:
        candidate='A7!b'+secrets.token_hex(24)
        if '123456' not in candidate:return candidate
credentials={key:value() for key in ['admin_token','matrix_token','gotify_token','mattermost_token','smtp_password','proxy_password','failure_token']}
credentials.update({key:password() for key in ['admin_password','browser_password','browser_new_password','operator_password','operator_new_password']})
credentials.update(postgres_password=secrets.token_hex(32),proxy_username=secrets.token_hex(12),smtp_username=secrets.token_hex(12),telegram_token=str(secrets.randbelow(10**9))+':'+secrets.token_urlsafe(32))
(target/'version.txt').write_text(tomllib.loads((root/'Cargo.toml').read_text())['package']['version'])
(target/'credentials.json').write_text(json.dumps(credentials));(target/'credentials.json').chmod(0o600)
lines=[f'RELAYIX_TEST_{key.upper()}={value}' for key,value in credentials.items()]
lines+=['RELAYIX_TEST_BASE_URL=http://relayix.test:8080',f'RELAYIX_TEST_RUNTIME_DIR={target}']
(target/'compose.env').write_text('\n'.join(lines)+'\n');(target/'compose.env').chmod(0o600)
if os.environ.get('GITHUB_ACTIONS')=='true':
    for value in credentials.values():print('::add-mask::'+value)
print('Per-run test credentials generated in private temporary storage.')
