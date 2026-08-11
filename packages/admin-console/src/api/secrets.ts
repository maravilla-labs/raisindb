// SPDX-License-Identifier: BSL-1.1

//! Typed client for the secret store (`/api/secrets/{repo}/{branch}`).
//!
//! # Values go in; they never come out
//!
//! There is deliberately no read endpoint, so there is no function here that
//! returns a value, and no type below has a field that could hold one. Writes
//! take a plaintext and answer with `{ name, version, reference }`. If a UI
//! ever wants to show a value, the answer is that it cannot — the server does
//! not have a route for it.
//!
//! # Names contain slashes
//!
//! The auto-vault write path mints `node/{node_id}/{field.path}` for a schema
//! field marked `encrypted: true`, so a name is not one path segment. The
//! server captures it as an axum wildcard (`{*name}`), which means the slashes
//! must arrive as REAL slashes — a `%2F` would not match the wildcard the same
//! way. Hence `encodeName`, which encodes each segment and rejoins on `/`
//! rather than encoding the whole string.
//!
//! For the same reason rotate is `POST .../rotate/{*name}` and not
//! `POST .../{name}/rotate`: an axum wildcard must be the last segment.

import { api } from './client'

/** Storage revision — ordering truth. The `version` ordinal is only a label. */
export interface SecretRevision {
  timestamp_ms: number
  counter: number
}

/**
 * Everything about one secret version EXCEPT the secret.
 *
 * Mirrors the server's `SecretMetadata`, which has no field able to carry
 * ciphertext or plaintext. `ciphertext_len` is the sealed envelope's SIZE, not
 * its content, and is safe to display.
 */
export interface SecretMetadata {
  name: string
  version: number
  key_id: number
  created_at: string
  created_by: string
  rotated_at?: string | null
  /** The node this secret backs, when it is a vaulted schema field. */
  owner_node?: string | null
  /** The dot path of the field it backs. */
  owner_field?: string | null
  /** Tombstone marker. Prior versions stay resolvable through a pinned ref. */
  deleted: boolean
  revision: SecretRevision
  ciphertext_len: number
}

/** What a write reports back. Carries no secret material by construction. */
export interface SecretWriteResponse {
  name: string
  version: number
  /** `secret://{name}` — unpinned, so a property using it follows rotations. */
  reference: string
}

export interface SecretVersionsResponse {
  name: string
  /** Newest first. Tombstones included and flagged, never filtered out. */
  versions: SecretMetadata[]
  /** Whether the newest version is a tombstone. */
  deleted: boolean
}

export interface SecretDeleteResponse {
  name: string
  version: number
  deleted: boolean
}

const base = (repo: string, branch: string) =>
  `/api/secrets/${encodeURIComponent(repo)}/${encodeURIComponent(branch)}`

/** Per-segment encoding — see the module docs on why `encodeURIComponent(name)`
 *  alone would be wrong. */
function encodeName(name: string): string {
  return name.split('/').map(encodeURIComponent).join('/')
}

export const secretsApi = {
  /** Newest version of every secret in the branch. Metadata only. */
  async list(repo: string, branch: string): Promise<SecretMetadata[]> {
    const res = await api.get<{ secrets: SecretMetadata[] }>(base(repo, branch))
    return res.secrets || []
  },

  /** Every version of one secret, newest first. Metadata only. */
  versions(repo: string, branch: string, name: string): Promise<SecretVersionsResponse> {
    return api.get<SecretVersionsResponse>(`${base(repo, branch)}/${encodeName(name)}`)
  },

  /** Create, or append a version. Write-only: nothing reads the value back. */
  put(repo: string, branch: string, name: string, value: string): Promise<SecretWriteResponse> {
    return api.put<SecretWriteResponse>(`${base(repo, branch)}/${encodeName(name)}`, { value })
  },

  /**
   * Append a version stamped `rotated_at`.
   *
   * An append, not a replacement: anything holding a pinned `secret://name@N`
   * — an older node revision, a running flow — keeps resolving.
   */
  rotate(repo: string, branch: string, name: string, value: string): Promise<SecretWriteResponse> {
    return api.post<SecretWriteResponse>(`${base(repo, branch)}/rotate/${encodeName(name)}`, {
      value,
    })
  },

  /**
   * Append a tombstone. Prior versions are never removed, so time-travel reads
   * of older node revisions still resolve their pinned references.
   */
  remove(repo: string, branch: string, name: string): Promise<SecretDeleteResponse> {
    return api.delete<SecretDeleteResponse>(`${base(repo, branch)}/${encodeName(name)}`)
  },
}

/** The owner encoded in an auto-vaulted name, or null for an operator secret. */
export interface SecretOwnerRef {
  nodeId: string
  field: string
}

/**
 * Split `node/{node_id}/{field.path}` into its parts.
 *
 * Prefers the metadata the server recorded (`owner_node` / `owner_field`) and
 * falls back to parsing the name, so a secret written before those fields were
 * populated still renders as owned rather than as an opaque string. The field
 * path may itself contain dots (`venue.geo.token`) but never a slash, so
 * everything after the second segment is the field.
 */
export function ownerOf(secret: SecretMetadata): SecretOwnerRef | null {
  if (secret.owner_node) {
    return { nodeId: secret.owner_node, field: secret.owner_field || '' }
  }
  const parts = secret.name.split('/')
  if (parts.length >= 3 && parts[0] === 'node' && parts[1]) {
    return { nodeId: parts[1], field: parts.slice(2).join('/') }
  }
  return null
}

/** An auto-vaulted secret is one a node owns; everything else is operator-made. */
export function isAutoVaulted(secret: SecretMetadata): boolean {
  return ownerOf(secret) !== null
}
