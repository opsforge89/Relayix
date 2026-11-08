"""User-facing 0.8.2 flows against the isolated API and local delivery mocks."""
import copy
import json
import os
import re
import time
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
from runtime import credentials
BASE=os.environ['RELAYIX_TEST_BASE_URL']
OUT=Path(os.environ.get('RELAYIX_TEST_ARTIFACTS','/artifacts'));OUT.mkdir(parents=True,exist_ok=True)
with sync_playwright() as p:
 browser=p.chromium.launch()
 page=browser.new_page(viewport={'width':1680,'height':1200},device_scale_factor=1,locale='en-US')
 page.set_default_timeout(12000)
 errors=[];page.on('pageerror',lambda error:errors.append(str(error)))
 api=p.request.new_context(base_url=BASE,extra_http_headers={'Authorization':'Bearer '+credentials['admin_token'],'X-Relay-Client':'web'})
 channels=[];routes=[];templates=[]
 def ok(response):assert response.ok,response.text();return response.json()
 def nav(name):page.locator('nav').get_by_role('button',name=name,exact=True).click()
 def row(name):return page.get_by_role('table',name='Templates',exact=True).locator('tbody tr').filter(has=page.get_by_text(name,exact=True))
 settings=None;preferences=None
 setup=p.request.new_context(base_url=BASE,extra_http_headers={'X-Relay-Client':'web'})
 try:
  ok(setup.post('/api/auth/login',data={'username':'admin','password':credentials['admin_password']}))
  preferences=ok(setup.get('/api/auth/me'))['preferences']
  # Earlier browser scenarios deliberately persist filters; this suite owns its own fixtures.
  ok(setup.put('/api/auth/preferences',data={**preferences,'list_search':'','route_tag_filter':'','page_size':50,'delivery_filters':{key:'' for key in ['status','channel_id','route_id','event_id','from','to','search','tag','period']}}))
  for name,path in [('082 UI failed destination','fail'),('082 UI independent alerts','ui082-alerts')]:
   channels.append(ok(api.post('/api/channels',data={'name':name,'kind':'webhook','enabled':True,'config':{'url':'http://mock:8000/'+path}}))['id'])
  original=ok(api.post('/api/routes',data={'name':'082 UI route','channel_ids':[channels[0]]}));routes.append(original['id'])
  page.goto(BASE)
  page.get_by_label('Username',exact=True).fill('admin');page.get_by_label('Password',exact=True).fill(credentials['admin_password']);page.get_by_role('button',name='Sign in',exact=True).click()
  expect(page.get_by_role('heading',name='Dashboard',exact=True)).to_be_visible()
  nav('Templates');page.get_by_role('button',name='Use Test results example',exact=True).click()
  editor=page.get_by_role('dialog',name='Create template',exact=True)
  expect(editor.get_by_label('Compute status from test counts',exact=True)).to_be_checked()
  editor.get_by_label('Template name',exact=True).fill('082 UI template')
  editor.get_by_label('Title (optional)',exact=True).fill('')
  editor.get_by_label('Message template',exact=True).fill('{{status}}')
  editor.get_by_role('button',name='Preview',exact=True).click()
  expect(editor.get_by_role('region',name='Template preview')).to_contain_text('Computed status: ❌')
  sample={'suite':'UnitTests','passed':40,'failed':0}
  editor.get_by_label('Example incoming JSON',exact=True).fill(json.dumps(sample));editor.get_by_role('button',name='Preview',exact=True).click()
  expect(editor.get_by_role('region',name='Template preview')).to_contain_text('Computed status: ✅')
  del sample['failed'];editor.get_by_label('Example incoming JSON',exact=True).fill(json.dumps(sample));editor.get_by_role('button',name='Preview',exact=True).click()
  expect(editor.get_by_role('region',name='Template preview')).to_contain_text('Computed status: ⚠️')
  expect(editor.get_by_role('region',name='Template preview')).to_contain_text('expected a non-negative integer')
  with page.expect_response(lambda r:r.url.endswith('/api/templates') and r.request.method=='POST') as created:
   editor.get_by_role('button',name='Save template',exact=True).click()
  template=ok(created.value);templates.append(template['id']);expect(editor).not_to_be_visible()
  row('082 UI template').get_by_role('button',name='Edit',exact=True).click()
  edit=page.get_by_role('dialog',name='Edit template',exact=True);edit.get_by_label('Message template',exact=True).fill('Updated {{status}}');edit.get_by_role('button',name='Save template',exact=True).click();expect(edit).not_to_be_visible()
  row('082 UI template').get_by_role('button',name='History',exact=True).click()
  history=page.get_by_role('dialog',name='Template version history',exact=True)
  history.locator('.version-list').get_by_role('button',name=re.compile('^Version 1')).click()
  expect(history.locator('.version-changed')).to_contain_text(['Message'])
  page.screenshot(path=str(OUT/'082-template-history.png'),full_page=True)
  history.get_by_role('button',name='Restore version 1',exact=True).click();expect(history).not_to_be_visible()
  assert next(t for t in ok(api.get('/api/templates')) if t['id']==template['id'])['version']==3
  row('082 UI template').get_by_role('button',name='Clone',exact=True).click()
  expect(editor.get_by_label('Template name',exact=True)).to_have_value('082 UI template copy')
  expect(editor.get_by_label('Compute status from test counts',exact=True)).to_be_checked()
  with page.expect_response(lambda r:r.url.endswith('/api/templates') and r.request.method=='POST') as copied:
   editor.get_by_role('button',name='Save template',exact=True).click()
  copy_template=ok(copied.value);templates.append(copy_template['id']);assert copy_template['version']==1
  nav('Routes')
  page.get_by_role('button',name='Edit route 082 UI route',exact=True).click()
  route_editor=page.get_by_role('dialog',name='Edit route',exact=True)
  route_editor.get_by_role('button',name='Use test results rules',exact=True).click()
  expect(route_editor.get_by_label('Validation field 1',exact=True)).to_have_value('suite')
  expect(route_editor.get_by_label('Validation minimum 2',exact=True)).to_have_value('0')
  route_editor.get_by_label('Message template (optional)',exact=True).select_option(template['id'])
  page.screenshot(path=str(OUT/'082-route-validation.png'),full_page=True)
  route_editor.get_by_role('button',name='Save',exact=True).click();expect(route_editor).not_to_be_visible()
  page.get_by_role('button',name='Clone route 082 UI route',exact=True).click()
  clone_editor=page.get_by_role('dialog',name='Add route',exact=True)
  expect(clone_editor.get_by_label('Name',exact=True)).to_have_value('082 UI route copy')
  expect(clone_editor.get_by_label('Validation field 3',exact=True)).to_have_value('failed')
  with page.expect_response(lambda r:r.url.endswith('/api/routes') and r.request.method=='POST') as cloned:
   clone_editor.get_by_role('button',name='Save',exact=True).click()
  cloned_route=ok(cloned.value);routes.append(cloned_route['id']);assert cloned_route['token']!=original['token'] and cloned_route['endpoint']!=original['endpoint']
  page.get_by_role('dialog',name='Save your route key',exact=True).get_by_role('button',name='I saved the key',exact=True).click()
  # Structured validation reaches the sender before any event is accepted.
  invalid=api.post(original['endpoint'],headers={'X-Relay-Key':original['token']},data={'passed':40,'failed':'wrong'})
  assert invalid.status==400 and 'suite' in invalid.json()['fields']
  page.get_by_role('button',name='Account menu',exact=True).click();page.locator('.profile-dropdown').get_by_role('button',name='My profile',exact=True).click()
  personal=page.locator('.personal-alerts')
  expect(personal.get_by_role('button',name='Save delivery alerts',exact=True)).to_be_visible()
  settings=page.request.get(BASE+'/api/auth/delivery-alerts',headers={'X-Relay-Client':'web'}).json()['policy']
  personal.get_by_label('Receive delivery failure alerts',exact=True).check()
  personal.get_by_role('button',name='Choose alert recipients',exact=True).click()
  picker=page.get_by_role('dialog',name='Choose alert recipients',exact=True)
  picker.get_by_label('Search recipients',exact=True).fill('082 UI independent alerts');picker.get_by_label('082 UI independent alerts',exact=True).check();picker.get_by_role('button',name='Done',exact=True).click()
  personal.get_by_label('When to notify',exact=True).select_option('final')
  with page.expect_response(lambda r:r.url.endswith('/api/auth/delivery-alerts') and r.request.method=='PUT') as saved:
   personal.get_by_role('button',name='Save delivery alerts',exact=True).click()
  assert saved.value.ok
  personal.get_by_role('button',name='Send test alert',exact=True).click()
  expect(personal).to_contain_text('Test alert queued')
  personal.get_by_role('button',name='Refresh alert history',exact=True).click()
  expect(personal.get_by_role('table',name='My alert history')).to_contain_text('082 UI independent alerts')
  page.screenshot(path=str(OUT/'082-personal-delivery-alerts.png'),full_page=True)
  event=ok(api.post(original['endpoint'],headers={'X-Relay-Key':original['token']},data={'suite':'UnitTests','passed':40,'failed':2}))
  nav('Deliveries');page.get_by_label('Route',exact=True).select_option(original['id']);page.get_by_label('Status',exact=True).select_option('failed');page.get_by_role('button',name='Apply filters',exact=True).click()
  page.get_by_role('button',name='Retry failed deliveries',exact=True).click()
  batch=page.get_by_role('dialog',name='Retry failed deliveries',exact=True)
  deadline=time.monotonic()+60
  while not (batch.get_by_role('button',name='Retry 1 deliveries',exact=True).count() and batch.get_by_role('button',name='Retry 1 deliveries',exact=True).is_enabled()):
   if time.monotonic()>deadline:raise AssertionError('Failed UI fixture did not reach retry preview')
   page.wait_for_timeout(1500);batch.get_by_role('button',name='Refresh preview',exact=True).click()
  page.screenshot(path=str(OUT/'082-batch-retry.png'),full_page=True)
  with page.expect_response(lambda r:'/api/deliveries/retry-batch/' in r.url and not r.url.endswith('/preview') and r.request.method=='POST') as queued:
   batch.get_by_role('button',name='Retry 1 deliveries',exact=True).click()
  assert ok(queued.value)['queued']==1;expect(batch).not_to_be_visible()
  for name in ['Routes','Templates','My profile']:
   page.set_viewport_size({'width':390,'height':844})
   if name=='My profile':
    page.get_by_role('button',name='Account menu',exact=True).click();page.locator('.profile-dropdown').get_by_role('button',name='My profile',exact=True).click()
   else:nav(name)
   expect(page.get_by_role('heading',name=name,exact=True)).to_be_visible()
   assert page.evaluate('document.documentElement.scrollWidth<=window.innerWidth'),name+' overflow'
  assert not errors,errors
  print('PASS: 0.8.2 status preview/warnings, template comparison/restore/clone, route validation/clone, profile subscriptions/test/history, bulk retry confirmation and mobile layouts',flush=True)
 finally:
  # Personal settings are account-owned; restore the previous values without using the administrator API identity.
  if page.get_by_role('button',name='Account menu').count():
   previous={'enabled':False,'destination_ids':[],'watch_route_ids':[],'trigger_mode':'final','failure_threshold':3,'window_minutes':15,'cooldown_minutes':15}
   if settings:previous={key:settings[key] for key in ['enabled','destination_ids','watch_route_ids','failure_threshold','window_minutes','cooldown_minutes']};previous['trigger_mode']='final' if settings['final_only'] else 'threshold' if settings['failure_threshold']>1 else 'first_error'
   response=page.request.put(BASE+'/api/auth/delivery-alerts',headers={'X-Relay-Client':'web'},data=previous)
   assert response.ok,response.text()
  for rid in routes:ok(api.delete('/api/routes/'+rid))
  for tid in templates:ok(api.delete('/api/templates/'+tid))
  for cid in channels:ok(api.delete('/api/channels/'+cid))
  api.dispose();browser.close()
  if preferences is not None:ok(setup.put('/api/auth/preferences',data=preferences))
  setup.dispose()
