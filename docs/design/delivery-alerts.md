# Personal delivery failure alerts

Status: implemented in 0.8.2 after deferral from 0.8.1.

My profile → Delivery alerts provides account-owned subscriptions, enabled channel recipients, optional route filters, final-failure/first-error/threshold triggers, cooldown, test notifications, private history and retry of failed warnings.

The implementation reuses the separate alert queue, adds explicit ownership and final-state matching, and deduplicates incidents by delivery and retry generation. Shared administrator rules and history remain separate. Disabled accounts and subscriptions cannot send queued personal warnings. Alert failures never trigger new alerts.

See [the user guide and API](../REFERENCE.md#personal-delivery-alerts) for current behavior. The original design comparison remains in [interface-options.html](interface-options.html). Recovery notifications remain a future enhancement.
