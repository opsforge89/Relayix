"""Shared browser interactions for compact channel and route action menus."""
import re


def menu_action(scope, name):
    """Open the named resource's menu and return its visible action."""
    for prefix in ('Edit route ', 'Clone route ', 'Rotate key ', 'Test webhook ', 'Copy endpoint ', 'Delete route '):
        if name.startswith(prefix):
            route_name = name[len(prefix):]
            scope = scope.locator('.route-row').filter(has=scope.get_by_role('button', name=route_name, exact=True)) if not scope.locator('.resource-actions').count() == 1 else scope
            break
    scope.get_by_role('button', name=re.compile(r'^(Channel|Route) actions ')).first.click()
    label = {'Diagnose': 'Diagnostics', 'Test': 'Send test'}.get(name, name)
    return scope.get_by_role('menuitem', name=label, exact=True)
