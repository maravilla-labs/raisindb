# Changelog

## Unreleased

### Security

- **A signed-in user could grant themselves roles and groups.** The built-in
  `authenticated_user` role allowed `update` on the caller's own `raisin:User`
  node with no field limit, and a user's direct roles and groups are read from
  that node's `roles` / `groups` properties. The self update is now limited to
  `display_name`, `title` and `metadata`. Existing repositories are fixed on
  engine start by a security system update
  (`raisin_core::system_updates::role_security`), whatever the
  `auto_apply` policy says: only the open self-update permission of the role
  is rewritten, a customised role keeps every other permission, and a role
  already fixed is not written again.
- Field-level write grants: `fields` / `except_fields` now restrict what a
  `create` or `update` grant may write (properties, translations by property
  key; archetype, owner, relations, path, type and publication only with an
  unrestricted grant). An update grant's `condition` is judged on the node as
  stored, so a write cannot satisfy it by setting the values it checks.

### Fixed

- Partial updates: `UPDATE … SET properties = properties || {…}` and the
  functions' `nodes.update` write only the named properties, against the node
  as stored when the write commits; a concurrent write of another property is
  no longer lost. `nodes.patch` removes a property set to `null`;
  `nodes.update(…, { replace: true })` replaces the whole map.
- Completing an inbox task is announced by its commit (node data, changed
  properties), so triggers on `raisin:InboxTask` updates fire reliably.
- A package install never narrows a workspace and widens both
  `allowed_node_types` and `allowed_root_node_types` in every mode.

### Added

- `raisin.http.fetch(url, { redirect: "manual" })` (and `followRedirects:
  false`) returns a 3xx response instead of following it.
- `raisin.assets.readBytes(workspace, path, property?)` / Rust SDK
  `raisin_sdk::assets::read_bytes`: an asset's bytes, read with the caller's
  rights.
