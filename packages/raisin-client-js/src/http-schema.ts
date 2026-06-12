/**
 * HTTP schema-management operations (NodeTypes / Archetypes / ElementTypes)
 *
 * These classes give the HTTP (SSR) client the same schema surface as the
 * WebSocket `Database`, backed by the management REST API
 * (`/api/management/{repo}/{branch}/...`). They mirror the WebSocket SDK
 * method names so call sites can swap clients with minimal changes.
 */

import type { RaisinHttpClient } from './http-client';

/** Optional commit metadata for write operations. Server-side it is optional
 *  (falls back to a default message + the `"system"` actor). */
export interface SchemaCommit {
  message: string;
  actor?: string;
}

type SchemaKind = 'nodetypes' | 'archetypes' | 'elementtypes';
type SchemaDefField = 'node_type' | 'archetype' | 'element_type';

/**
 * Shared CRUD/publish/resolve implementation for the three schema kinds.
 */
abstract class HttpSchemaBase {
  protected abstract readonly kind: SchemaKind;
  protected abstract readonly defField: SchemaDefField;

  constructor(
    protected readonly repository: string,
    protected readonly branch: string,
    protected readonly client: RaisinHttpClient
  ) {}

  /** Base management URL for this schema kind. */
  protected base(): string {
    return `/api/management/${this.repository}/${this.branch}/${this.kind}`;
  }

  /** List all schemas (or only published ones). */
  async list(publishedOnly = false): Promise<unknown[]> {
    const path = publishedOnly ? `${this.base()}/published` : this.base();
    const result = await this.client.managementRequest<unknown>('GET', path);
    return Array.isArray(result) ? result : [];
  }

  /** Get a single schema by name. */
  async get(name: string): Promise<unknown> {
    return this.client.managementRequest('GET', `${this.base()}/${encodeURIComponent(name)}`);
  }

  /** Get the schema with full inheritance (`extends`) applied. */
  async getResolved(name: string): Promise<unknown> {
    return this.client.managementRequest(
      'GET',
      `${this.base()}/${encodeURIComponent(name)}/resolved`
    );
  }

  /** Create a schema. `name` is folded into the definition body if absent. */
  async create(
    name: string,
    definition: Record<string, unknown>,
    commit?: SchemaCommit
  ): Promise<unknown> {
    return this.client.managementRequest('POST', this.base(), {
      [this.defField]: { name, ...definition },
      commit,
    });
  }

  /** Update an existing schema. */
  async update(
    name: string,
    definition: Record<string, unknown>,
    commit?: SchemaCommit
  ): Promise<unknown> {
    return this.client.managementRequest(
      'PUT',
      `${this.base()}/${encodeURIComponent(name)}`,
      { [this.defField]: { name, ...definition }, commit }
    );
  }

  /** Delete a schema. */
  async delete(name: string, commit?: SchemaCommit): Promise<unknown> {
    return this.client.managementRequest(
      'DELETE',
      `${this.base()}/${encodeURIComponent(name)}`,
      commit
    );
  }

  /** Publish a schema. */
  async publish(name: string, commit?: SchemaCommit): Promise<unknown> {
    return this.client.managementRequest(
      'POST',
      `${this.base()}/${encodeURIComponent(name)}/publish`,
      commit
    );
  }

  /** Unpublish a schema. */
  async unpublish(name: string, commit?: SchemaCommit): Promise<unknown> {
    return this.client.managementRequest(
      'POST',
      `${this.base()}/${encodeURIComponent(name)}/unpublish`,
      commit
    );
  }
}

/** NodeType management over HTTP REST. */
export class HttpNodeTypes extends HttpSchemaBase {
  protected readonly kind: SchemaKind = 'nodetypes';
  protected readonly defField: SchemaDefField = 'node_type';

  /**
   * Get the resolved NodeType. Unlike archetypes/element types, NodeType
   * resolution can be workspace-aware (honors workspace NodeType pins).
   */
  async getResolved(name: string, opts?: { workspace?: string }): Promise<unknown> {
    const query = opts?.workspace ? `?workspace=${encodeURIComponent(opts.workspace)}` : '';
    return this.client.managementRequest(
      'GET',
      `${this.base()}/${encodeURIComponent(name)}/resolved${query}`
    );
  }
}

/** Archetype management over HTTP REST. */
export class HttpArchetypes extends HttpSchemaBase {
  protected readonly kind: SchemaKind = 'archetypes';
  protected readonly defField: SchemaDefField = 'archetype';
}

/** ElementType management over HTTP REST. */
export class HttpElementTypes extends HttpSchemaBase {
  protected readonly kind: SchemaKind = 'elementtypes';
  protected readonly defField: SchemaDefField = 'element_type';
}
