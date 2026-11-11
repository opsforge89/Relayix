"""Owner sharing controls, independent permissions and administrator visibility in Chromium."""
import os
import uuid
from playwright.sync_api import sync_playwright, expect
from runtime import credentials

BASE = os.environ['RELAYIX_TEST_BASE_URL']
with sync_playwright() as p:
    browser = p.chromium.launch()
    api = p.request.new_context(base_url=BASE, extra_http_headers={'Authorization': 'Bearer ' + credentials['admin_token'], 'X-Relay-Client': 'web'})
    def ok(response):
        assert response.ok, 'Access browser fixture failed: ' + str(response.status)
        return response.json()
    users, channels = [], []
    pages = []
    errors = []
    suffix = uuid.uuid4().hex[:16]
    try:
        for label in ['owner', 'reader']:
            name = 'access-ui-' + label + '-' + suffix
            users.append(ok(api.post('/api/users', data={'username': name, 'display_name': label, 'role': 'member', 'password': credentials['operator_password']}))['id'])
            page = browser.new_page(viewport={'width': 1680, 'height': 1100})
            page.set_default_timeout(15000)
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.goto(BASE)
            page.get_by_label('Username', exact=True).fill(name)
            page.get_by_label('Password', exact=True).fill(credentials['operator_password'])
            page.get_by_role('button', name='Sign in', exact=True).click()
            expect(page.get_by_role('heading', name='Dashboard', exact=True)).to_be_visible()
            pages.append(page)
        owner, reader = pages
        cid = ok(owner.request.post(BASE + '/api/channels', headers={'X-Relay-Client': 'web'}, data={'name': 'Private UI ' + suffix, 'kind': 'webhook', 'config': {'url': 'http://mock:8000/acl-ui-' + suffix}}))['id']
        channels.append(cid)
        assert any(c['id'] == cid for c in ok(api.get('/api/channels')))
        for page in pages:
            page.reload()
            expect(page.get_by_role('heading', name='Dashboard', exact=True)).to_be_visible()
            page.locator('nav').get_by_role('button', name='Channels', exact=True).click()
        expect(reader.locator('.channel-entry')).to_have_count(0)
        card = owner.locator('.channel-entry').filter(has=owner.get_by_text('Private UI ' + suffix, exact=True))
        card.get_by_role('button', name='Access', exact=True).click()
        dialog = owner.get_by_role('dialog', name='Channel access', exact=True)
        expect(dialog.get_by_label('Channel owner', exact=True)).to_be_disabled()
        dialog.get_by_label('Add collaborator', exact=True).select_option(users[1])
        dialog.get_by_role('button', name='Add', exact=True).click()
        dialog.get_by_role('button', name='Save', exact=True).click()
        expect(dialog).not_to_be_visible()
        reader.get_by_role('button', name='Refresh data', exact=True).click()
        shared = reader.locator('.channel-entry').filter(has=reader.get_by_text('Private UI ' + suffix, exact=True))
        expect(shared).to_be_visible()
        expect(shared.get_by_role('button', name='View', exact=True)).to_be_visible()
        for name in ['Edit', 'Test', 'Access', 'Pause', 'Diagnose']:
            expect(shared.get_by_role('button', name=name, exact=True)).to_have_count(0)
        shared.get_by_role('button', name='View', exact=True).click()
        settings = reader.get_by_role('dialog', name='Channel settings', exact=True)
        expect(settings).to_contain_text('********')
        expect(settings).not_to_contain_text('mock:8000')
        settings.get_by_role('button', name='Close dialog', exact=True).click()
        card.get_by_role('button', name='Access', exact=True).click()
        dialog = owner.get_by_role('dialog', name='Channel access', exact=True)
        dialog.get_by_role('button', name='Remove', exact=True).click()
        dialog.get_by_role('button', name='Save', exact=True).click()
        expect(dialog).not_to_be_visible()
        reader.get_by_role('button', name='Refresh data', exact=True).click()
        expect(shared).to_have_count(0)
        assert not errors, errors
        print('PASS: private channel UI, owner sharing, read-only settings and revoked visibility')
    finally:
        for cid in channels: api.delete('/api/channels/' + cid)
        for uid in users: api.delete('/api/users/' + uid)
        browser.close()
