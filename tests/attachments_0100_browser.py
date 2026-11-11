"""Original binary attachments are discoverable and privately downloadable in the browser."""
import json
import os
import uuid
from playwright.sync_api import sync_playwright,expect
from runtime import credentials
BASE=os.environ['RELAYIX_TEST_BASE_URL']
with sync_playwright() as p:
    browser=p.chromium.launch();page=browser.new_page();page.set_default_timeout(15000)
    errors=[];page.on('pageerror',lambda error:errors.append(str(error)))
    api=p.request.new_context(base_url=BASE,extra_http_headers={'Authorization':'Bearer '+credentials['admin_token'],'X-Relay-Client':'web'})
    def ok(response):assert response.ok,response.text();return response.json()
    suffix=uuid.uuid4().hex;user=channel=route=None
    blob=b'PK\x00\xff'+suffix.encode()
    try:
        username='file-browser-'+suffix
        user=ok(api.post('/api/users',data={'username':username,'display_name':'File browser','role':'member','password':credentials['operator_password']}))['id']
        channel=ok(api.post('/api/channels',data={'name':'File browser channel','kind':'mattermost','config':{'mode':'bot','url':'http://mock:8000','target':'channel','channel_id':'browserfiles','bot_token':credentials['mattermost_token']}}))['id']
        ok(api.put('/api/channels/'+channel+'/access',data={'owner_user_id':None,'grants':[{'user_id':user,'can_view':True,'can_edit':False,'can_send':False}]}))
        route=ok(api.post('/api/routes',data={'name':'File browser '+suffix,'channel_ids':[channel]}))
        ok(api.post('/api/channels/'+channel+'/pause',data={}))
        accepted=ok(api.post(route['endpoint']+'/files',headers={'X-Relay-Key':route['token']},multipart={'payload':json.dumps({'title':'File history '+suffix,'message':'Preserved report'}),'files':{'name':'report.xlsx','mimeType':'application/octet-stream','buffer':blob}}))
        page.goto(BASE);page.get_by_label('Username',exact=True).fill(username);page.get_by_label('Password',exact=True).fill(credentials['operator_password']);page.get_by_role('button',name='Sign in',exact=True).click()
        expect(page.get_by_role('heading',name='Dashboard',exact=True)).to_be_visible()
        page.locator('nav').get_by_role('button',name='Incoming webhooks',exact=True).click()
        page.get_by_label('Search events',exact=True).fill('File history '+suffix)
        row=page.get_by_role('row').filter(has=page.get_by_role('button',name='File history '+suffix,exact=True))
        row.get_by_role('button',name='Details',exact=True).click()
        section=page.locator('section[aria-label="Attachments"]')
        expect(section).to_contain_text('report.xlsx')
        with page.expect_download() as download:
            section.get_by_role('button',name='report.xlsx',exact=True).click()
        from pathlib import Path
        assert Path(download.value.path()).read_bytes()==blob
        assert download.value.suggested_filename=='report.xlsx'
        assert ok(api.get('/api/events/'+accepted['event_id']))['attachments'][0]['size']==len(blob)
        assert not errors,errors
        print('PASS: authenticated member discovers original file metadata and downloads exact binary bytes from event history',flush=True)
    finally:
        if route:api.delete('/api/routes/'+route['id'])
        if channel:api.delete('/api/channels/'+channel)
        if user:api.delete('/api/users/'+user)
        browser.close()
