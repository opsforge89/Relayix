"""Exercise JSON body editing, route quotas and admin configuration transfer in Chromium."""
import json
import os
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
from runtime import credentials

BASE=os.environ['RELAYIX_TEST_BASE_URL']
with sync_playwright() as p:
    browser=p.chromium.launch()
    page=browser.new_page(viewport={'width':1680,'height':1200},locale='en-US')
    page.set_default_timeout(15000)
    errors=[];page.on('pageerror',lambda error:errors.append(str(error)))
    api=p.request.new_context(base_url=BASE,extra_http_headers={'Authorization':'Bearer '+credentials['admin_token'],'X-Relay-Client':'web'})
    setup=p.request.new_context(base_url=BASE,extra_http_headers={'X-Relay-Client':'web'})
    def ok(r):assert r.ok,r.text();return r.json()
    def nav(name):page.locator('nav').get_by_role('button',name=name,exact=True).click()
    channels=[];routes=[];preferences=None
    try:
        ok(setup.post('/api/auth/login',data={'username':'admin','password':credentials['admin_password']}))
        preferences=ok(setup.get('/api/auth/me'))['preferences']
        ok(setup.put('/api/auth/preferences',data={**preferences,'list_search':'','route_tag_filter':''}))
        channel=ok(api.post('/api/channels',data={'name':'087 UI webhook body','kind':'webhook','enabled':False,'config':{'url':'http://mock:8000/ui087','body_template':{'text':'{{message}}'}}}))['id'];channels.append(channel)
        route=ok(api.post('/api/routes',data={'name':'087 UI rate limit','channel_ids':[channel]}));routes.append(route['id'])
        page.goto(BASE)
        page.get_by_label('Username',exact=True).fill('admin');page.get_by_label('Password',exact=True).fill(credentials['admin_password']);page.get_by_role('button',name='Sign in',exact=True).click()
        expect(page.get_by_role('heading',name='Dashboard',exact=True)).to_be_visible()
        nav('Channels')
        page.locator('.channel-entry').filter(has=page.get_by_text('087 UI webhook body',exact=True)).get_by_role('button',name='Edit',exact=True).click()
        editor=page.get_by_role('dialog',name='Edit channel',exact=True)
        editor.get_by_label('JSON body template (optional)',exact=True).fill('{invalid')
        editor.get_by_role('button',name='Save',exact=True).click();expect(editor).to_be_visible()
        body={'text':'{{message}}','severity':'{{priority}}'}
        editor.get_by_label('JSON body template (optional)',exact=True).fill(json.dumps(body))
        with page.expect_response(lambda r:r.url.endswith('/api/channels/'+channel) and r.request.method=='PUT') as saved:
            editor.get_by_role('button',name='Save',exact=True).click()
        ok(saved.value);expect(editor).not_to_be_visible()
        assert next(c for c in ok(api.get('/api/channels')) if c['id']==channel)['config']['body_template']==body
        nav('Routes')
        page.locator('.route-row').filter(has=page.get_by_text('087 UI rate limit',exact=True)).get_by_role('button',name='Edit',exact=True).click()
        editor=page.get_by_role('dialog',name='Edit route',exact=True)
        editor.get_by_label('Route rate limit per minute',exact=True).fill('3')
        with page.expect_response(lambda r:r.url.endswith('/api/routes/'+route['id']) and r.request.method=='PUT') as saved:
            editor.get_by_role('button',name='Save',exact=True).click()
        ok(saved.value);expect(editor).not_to_be_visible()
        assert next(r for r in ok(api.get('/api/routes')) if r['id']==route['id'])['rate_limit_per_minute']==3
        nav('Settings')
        with page.expect_download() as downloaded:page.get_by_role('button',name='Export configuration',exact=True).click()
        bundle=json.loads(Path(downloaded.value.path()).read_text())
        bundle['channels']=[c for c in bundle['channels'] if c['key']==channel]
        bundle['routes']=[r for r in bundle['routes'] if r['key']==route['id']]
        assert bundle['channels'] and bundle['routes'] and 'url' not in bundle['channels'][0]['config']
        before_channels={c['id'] for c in ok(api.get('/api/channels'))};before_routes={r['id'] for r in ok(api.get('/api/routes'))}
        page.locator('.configuration-transfer input[type=file]').set_input_files({'name':'configuration.json','mimeType':'application/json','buffer':json.dumps(bundle).encode()})
        with page.expect_response(lambda r:r.url.endswith('/api/config/import') and r.request.method=='POST') as imported:
            page.get_by_role('button',name='Import disabled drafts',exact=True).click()
        result=ok(imported.value);assert result['disabled']
        new_channels=[c for c in ok(api.get('/api/channels')) if c['id'] not in before_channels]
        new_routes=[r for r in ok(api.get('/api/routes')) if r['id'] not in before_routes]
        channels.extend(c['id'] for c in new_channels);routes.extend(r['id'] for r in new_routes)
        assert len(new_channels)==len(new_routes)==1 and not new_channels[0]['enabled'] and not new_routes[0]['enabled']
        assert new_routes[0]['channels']==[new_channels[0]['id']] and new_routes[0]['rate_limit_per_minute']==3
        assert not errors,errors
        print('PASS: Chromium body-template validation/save, route limit editing and disabled configuration import/export',flush=True)
    finally:
        for route in routes:api.delete('/api/routes/'+route)
        for channel in channels:api.delete('/api/channels/'+channel)
        if preferences:setup.put('/api/auth/preferences',data=preferences)
        browser.close()
