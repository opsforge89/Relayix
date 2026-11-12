"""Resource drawers retain dashboard context, keyboard access and mobile bounds."""
import os
import uuid
from pathlib import Path
from playwright.sync_api import sync_playwright, expect
from runtime import credentials

BASE = os.environ['RELAYIX_TEST_BASE_URL']
OUT = Path('/artifacts')
with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={'width': 1680, 'height': 1100}, locale='en-US')
    page.set_default_timeout(15000)
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    api = p.request.new_context(base_url=BASE, extra_http_headers={'Authorization': 'Bearer ' + credentials['admin_token'], 'X-Relay-Client': 'web'})
    setup = p.request.new_context(base_url=BASE, extra_http_headers={'X-Relay-Client': 'web'})
    def ok(response):
        assert response.ok, response.text()
        return response.json()
    channels, routes = [], []
    preferences = None
    suffix = uuid.uuid4().hex[:12]
    try:
        ok(setup.post('/api/auth/login', data={'username': 'admin', 'password': credentials['admin_password']}))
        preferences = ok(setup.get('/api/auth/me'))['preferences']
        ok(setup.put('/api/auth/preferences', data={**preferences, 'list_search': '', 'route_tag_filter': '', 'refresh_interval_seconds': 0}))
        for label in ['Tests', 'Deploy']:
            channel = ok(api.post('/api/channels', data={'name': label + ' drawer ' + suffix, 'kind': 'webhook', 'config': {'url': 'http://mock:8000/drawer-' + suffix}}))
            channels.append(channel['id'])
            route = ok(api.post('/api/routes', data={'name': label + ' route ' + suffix, 'channel_ids': [channel['id']]}))
            routes.append(route['id'])
        page.goto(BASE)
        page.get_by_label('Username', exact=True).fill('admin')
        page.get_by_label('Password', exact=True).fill(credentials['admin_password'])
        page.get_by_role('button', name='Sign in', exact=True).click()
        dashboard = page.locator('.dashboard-configuration')
        expect(dashboard.locator('.panel')).to_have_count(2)
        panels = dashboard.locator('.panel')
        assert abs(panels.nth(0).bounding_box()['y'] - panels.nth(1).bounding_box()['y']) < 2
        expect(dashboard.locator('.endpoint')).to_have_count(0)
        expect(dashboard.get_by_role('button', name='Edit', exact=True)).to_have_count(0)
        period = page.get_by_label('Dashboard period', exact=True).input_value()
        channel_name = page.get_by_role('button', name='Deploy drawer ' + suffix, exact=True)
        channel_name.scroll_into_view_if_needed()
        before = page.evaluate('window.scrollY')
        geometry = dashboard.bounding_box()
        channel_name.click()
        drawer = page.get_by_role('dialog', name='Channel details', exact=True)
        expect(drawer.get_by_role('tab', name='Overview', exact=True)).to_have_attribute('aria-selected', 'true')
        assert dashboard.bounding_box() == geometry
        assert abs(page.evaluate('window.scrollY') - before) < 1
        expect(page.get_by_role('heading', name='Dashboard', exact=True)).to_be_visible()
        page.screenshot(path=str(OUT / 'channel-overview-drawer.png'), full_page=True)
        drawer.get_by_role('tab', name='Settings', exact=True).click()
        drawer.get_by_label('Name', exact=True).fill('Unsaved drawer name')
        drawer.get_by_role('tab', name='Overview', exact=True).click()
        drawer.get_by_role('tab', name='Settings', exact=True).click()
        expect(drawer.get_by_label('Name', exact=True)).to_have_value('Unsaved drawer name')
        drawer.get_by_role('tab', name='Settings', exact=True).focus()
        page.keyboard.press('ArrowLeft')
        expect(drawer.get_by_role('tab', name='Overview', exact=True)).to_be_focused()
        trigger = drawer.get_by_role('button', name='Channel actions Deploy drawer ' + suffix, exact=True)
        trigger.click()
        expect(drawer.get_by_role('menu')).to_be_visible()
        page.keyboard.press('End')
        expect(drawer.get_by_role('menuitem', name='Delete channel Deploy drawer ' + suffix, exact=True)).to_be_focused()
        page.keyboard.press('Escape')
        expect(drawer).to_be_visible()
        expect(drawer.get_by_role('menu')).to_have_count(0)
        expect(trigger).to_be_focused()
        page.keyboard.press('Escape')
        expect(drawer).not_to_be_visible()
        expect(channel_name).to_be_focused()
        assert page.get_by_label('Dashboard period', exact=True).input_value() == period
        assert next(c for c in ok(api.get('/api/channels')) if c['id'] == channels[-1])['name'] == 'Deploy drawer ' + suffix
        route_name = page.get_by_role('button', name='Deploy route ' + suffix, exact=True)
        route_name.click()
        route_drawer = page.get_by_role('dialog', name='Route details', exact=True)
        expect(route_drawer.get_by_role('tab', name='Integration', exact=True)).to_be_visible()
        route_drawer.get_by_role('button', name='Test route', exact=True).click()
        tester = page.get_by_role('dialog', name='Test webhook', exact=True)
        expect(tester).to_be_visible()
        page.keyboard.press('Escape')
        expect(tester).not_to_be_visible()
        expect(route_drawer).to_be_visible()
        expect(route_drawer.get_by_role('button', name='Test route', exact=True)).to_be_focused()
        route_drawer.get_by_role('tab', name='Integration', exact=True).click()
        expect(route_drawer.locator('.endpoint')).to_contain_text('/h/')
        expect(route_drawer.get_by_role('button', name='Rotate key', exact=True)).to_be_visible()
        page.screenshot(path=str(OUT / 'route-integration-drawer.png'), full_page=True)
        page.mouse.click(10, 200)
        expect(route_drawer).not_to_be_visible()
        for theme in ['Light', 'Dark']:
            page.get_by_role('button', name=theme, exact=True).click()
            page.set_viewport_size({'width': 390, 'height': 844})
            for name, title in [('Deploy drawer ' + suffix, 'Channel details'), ('Deploy route ' + suffix, 'Route details')]:
                page.get_by_role('button', name=name, exact=True).click()
                mobile = page.get_by_role('dialog', name=title, exact=True)
                expect(mobile).to_be_visible()
                assert mobile.bounding_box()['width'] <= 390
                assert page.evaluate('document.documentElement.scrollWidth <= window.innerWidth')
                mobile.get_by_role('button', name='Close dialog', exact=True).click()
            page.set_viewport_size({'width': 1680, 'height': 1100})
        assert not errors, errors
        print('PASS: shared resource drawers, stable dashboard context, retained drafts, keyboard menus/tabs, nested focus, Integration and full-width mobile panels')
    finally:
        for route in routes:
            api.delete('/api/routes/' + route)
        for channel in channels:
            api.delete('/api/channels/' + channel)
        if preferences:
            setup.put('/api/auth/preferences', data=preferences)
        browser.close()
