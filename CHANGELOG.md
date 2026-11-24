# Changelog

## Unreleased

### Security

- **A signed-in user could grant themselves roles and groups.** The built-in
  `authenticated_user` role allowed `update` on the caller's own `raisin:User`
  node with no field limit, and a user's direct roles and groups are read from
  that node's `roles` / `groups` properties. Fixed three ways:
  - the shipped role limits the self update to `display_name`, `title` and
    `metadata`;
  - a security system update rewrites, on engine start, every `raisin:Role` on
    every branch of every repository that holds such a self grant (decided
    from the parsed permission, however it is spelled; only grants that name
    `raisin:User`, so an owner grant over all content is untouched), whatever the
    `auto_apply` policy says; other permissions are kept, a fixed role is not
    written again (`raisin_core::system_updates::role_security`);
  - the write layer refuses a non-admin caller's write of `roles`, `groups`,
    `can_login`, `status`, `email`, `email_verified`, `user_id`,
    `identity_id` or `birth_date` of their own user node, or a move of it,
    whatever any role grants (`rls_filter::self_guard`), on every write path
    including the NodeService property update.
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
