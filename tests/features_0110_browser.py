"""Administrator audit, credential-storage status and route TTL controls render in Chromium."""
import os
from playwright.sync_api import sync_playwright, expect
from runtime import credentials

BASE = os.environ['RELAYIX_TEST_BASE_URL']
with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page(viewport={'width': 1680, 'height': 1200})
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(BASE)
    page.get_by_label('Username', exact=True).fill('admin')
    page.get_by_label('Password', exact=True).fill(credentials['admin_password'])
    page.get_by_role('button', name='Sign in', exact=True).click()
    expect(page.get_by_role('heading', name='Dashboard', exact=True)).to_be_visible()
    page.locator('nav').get_by_role('button', name='Audit log', exact=True).click()
    expect(page.get_by_role('table', name='Audit log', exact=True)).to_be_visible()
    page.get_by_role('button', name='Refresh audit log', exact=True).click()
    expect(page.get_by_role('table', name='Audit log', exact=True)).to_contain_text('channel.create')
    page.locator('nav').get_by_role('button', name='Settings', exact=True).click()
    expect(page.get_by_role('heading', name='Stored credential encryption', exact=True)).to_be_visible()
    expect(page.get_by_text('Database credential encryption disabled', exact=True)).to_be_visible()
    page.locator('nav').get_by_role('button', name='Routes', exact=True).click()
    page.get_by_role('button', name='Add route', exact=True).click()
    expect(page.get_by_label('Message TTL (seconds)', exact=True)).to_have_value('0')
    page.get_by_label('Message TTL (seconds)', exact=True).fill('3600')
    expect(page.get_by_text('X-Relay-TTL may shorten this limit.', exact=False)).to_be_visible()
    assert errors == [], errors
    browser.close()
print('PASS: Chromium audit journal, encryption status and documented route TTL control')
