"""All delivery controls and opt-in administrator TOTP are visible and usable in Chromium."""
import base64
import hashlib
import hmac
import os
import struct
import time
import uuid
from playwright.sync_api import sync_playwright,expect
from runtime import credentials

BASE=os.environ['RELAYIX_TEST_BASE_URL']
def otp(secret):
    digest=hmac.new(base64.b32decode(secret+'='*((8-len(secret)%8)%8)),struct.pack('>Q',int(time.time())//30),hashlib.sha1).digest();offset=digest[-1]&15
    return str((struct.unpack('>I',digest[offset:offset+4])[0]&0x7fffffff)%1000000).zfill(6)
with sync_playwright() as p:
    browser=p.chromium.launch();page=browser.new_page(viewport={'width':1680,'height':1200},locale='en-US');page.set_default_timeout(15000)
    errors=[];page.on('pageerror',lambda error:errors.append(str(error)))
    api=p.request.new_context(base_url=BASE,extra_http_headers={'Authorization':'Bearer '+credentials['admin_token'],'X-Relay-Client':'web'})
    def ok(response):assert response.ok,response.text();return response.json()
    def nav(name):page.locator('nav').get_by_role('button',name=name,exact=True).click()
    users=[];channels=[];routes=[]
    suffix=uuid.uuid4().hex
    try:
        username='controls-'+suffix
        users.append(ok(api.post('/api/users',data={'username':username,'display_name':'Controls administrator','role':'admin','password':credentials['operator_password']}))['id'])
        channel=ok(api.post('/api/channels',data={'name':'090 UI channel','kind':'webhook','config':{'url':'http://mock:8000/ui090-'+suffix}}))['id'];channels.append(channel)
        route=ok(api.post('/api/routes',data={'name':'090 UI route','channel_ids':[channel]}));routes.append(route['id'])
        page.goto(BASE);page.get_by_label('Username',exact=True).fill(username);page.get_by_label('Password',exact=True).fill(credentials['operator_password']);page.get_by_role('button',name='Sign in',exact=True).click()
        expect(page.get_by_role('heading',name='Dashboard',exact=True)).to_be_visible()
        nav('Channels');card=page.locator('.channel-entry').filter(has=page.get_by_text('090 UI channel',exact=True))
        card.get_by_role('button',name='Pause',exact=True).click();dialog=page.get_by_role('dialog',name='Pause channel',exact=True)
        dialog.get_by_label('Pause duration',exact=True).select_option('0');dialog.get_by_role('button',name='Pause channel',exact=True).click()
        expect(card).to_contain_text('Paused manually');card.get_by_role('button',name='Resume',exact=True).click();expect(card).not_to_contain_text('Paused manually')
        nav('Routes');page.get_by_role('button',name='Edit route 090 UI route',exact=True).click();editor=page.get_by_role('dialog',name='Edit route',exact=True)
        editor.get_by_label('Quiet hours',exact=True).check();editor.get_by_label('Quiet-hours timezone',exact=True).fill('Europe/Moscow');editor.get_by_label('Deduplication window (seconds)',exact=True).fill('60');editor.get_by_label('Digest interval',exact=True).select_option('3600')
        editor.get_by_role('button',name='Save',exact=True).click();expect(editor).not_to_be_visible()
        saved=next(r for r in ok(api.get('/api/routes')) if r['id']==route['id'])
        assert saved['delivery_settings']['quiet_enabled'] and saved['delivery_settings']['dedup_seconds']==60 and saved['delivery_settings']['digest_seconds']==3600
        row=page.locator('.route-row').filter(has=page.get_by_text('090 UI route',exact=True));row.get_by_role('button',name='Integration guide',exact=True).click()
        guide=page.get_by_role('dialog',name='Route integration guide',exact=True)
        guide.get_by_role('tab',name='Python',exact=True).click();expect(guide.get_by_label('python integration example')).to_contain_text('urllib.request');expect(guide.get_by_label('python integration example')).to_contain_text(route['slug'])
        guide.get_by_role('tab',name='Bash',exact=True).click();expect(guide.get_by_label('bash integration example')).to_contain_text('set -euo pipefail');guide.get_by_role('button',name='Close dialog',exact=True).click()
        nav('Settings');security=page.locator('.totp-settings');expect(security).to_contain_text('Two-factor authentication disabled')
        security.get_by_label('Current password for two-factor setup',exact=True).fill(credentials['operator_password']);security.get_by_role('button',name='Set up authenticator',exact=True).click()
        key=security.get_by_label('Authenticator setup key').text_content()
        assert key and 'enabled' not in security.get_by_role('status').inner_text()
        security.get_by_label('Authenticator confirmation code',exact=True).fill(otp(key));security.get_by_role('button',name='Enable two-factor authentication',exact=True).click()
        expect(security.get_by_role('status')).to_have_text('Two-factor authentication enabled')
        recovery=security.get_by_label('One-time recovery codes').locator('pre').inner_text().splitlines();assert len(recovery)==10
        security.get_by_role('button',name='I saved the recovery codes',exact=True).click();page.reload();expect(page.locator('.totp-settings').get_by_role('status')).to_have_text('Two-factor authentication enabled')
        page.locator('.settings-actions').get_by_role('button',name='Sign out',exact=True).click()
        page.get_by_label('Username',exact=True).fill(username);page.get_by_label('Password',exact=True).fill(credentials['operator_password']);page.get_by_label('Authenticator or recovery code (if enabled)',exact=True).fill(recovery[0]);page.get_by_role('button',name='Sign in',exact=True).click()
        expect(page.get_by_role('heading',name='Dashboard',exact=True)).to_be_visible();nav('Settings');security=page.locator('.totp-settings')
        security.locator('summary').filter(has_text='Disable two-factor authentication').click();security.get_by_label('Current password to disable two-factor',exact=True).fill(credentials['operator_password']);security.get_by_label('Authenticator or recovery code',exact=True).fill(recovery[1]);security.get_by_role('button',name='Disable two-factor authentication',exact=True).click()
        expect(security.get_by_role('status')).to_have_text('Two-factor authentication disabled')
        assert not errors,errors
        print('PASS: pause/resume UI, quiet/grouping settings, Python/Bash examples and explicit TOTP enrollment, recovery login and disable',flush=True)
    finally:
        for user in users:api.post('/api/users/'+user+'/totp/reset',data={})
        for route_id in routes:api.delete('/api/routes/'+route_id)
        for channel_id in channels:api.delete('/api/channels/'+channel_id)
        for user in users:api.delete('/api/users/'+user)
        browser.close()
