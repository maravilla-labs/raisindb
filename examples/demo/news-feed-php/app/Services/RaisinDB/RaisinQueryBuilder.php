<?php

namespace App\Services\RaisinDB;

use Illuminate\Support\Facades\DB;

class RaisinQueryBuilder
{
    protected string $workspace = 'social';
    protected array $wheres = [];
    protected array $bindings = [];
    protected ?string $orderBy = null;
    protected ?int $limit = null;
    protected ?int $offset = null;
    protected array $columns = ['*'];

    /**
     * Current identity user token (JWT)
     * Set via middleware to enable row-level security
     */
    protected static ?string $userToken = null;

    public function __construct(string $workspace = 'social')
    {
        $this->workspace = $workspace;
    }

    /**
     * Create a new query builder instance
     */
    public static function query(string $workspace = 'social'): self
    {
        return new self($workspace);
    }

    /**
     * Set the identity user token for row-level security.
     * Call this from middleware after validating the JWT.
     */
    public static function setUserToken(?string $token): void
    {
        self::$userToken = $token;
    }

    /**
     * Get the current user token
     */
    public static function getUserToken(): ?string
    {
        return self::$userToken;
    }

    /**
     * Clear the user token (call at end of request)
     */
    public static function clearUserToken(): void
    {
        self::$userToken = null;
    }

    /**
     * Execute a callback with identity context.
     * Sets app.user before the callback and resets it after.
     */
    protected static function withUserContext(callable $callback)
    {
        if (self::$userToken) {
            DB::statement('SET app.user = ?', [self::$userToken]);
        }

        try {
            return $callback();
        } finally {
            if (self::$userToken) {
                DB::statement('RESET app.user');
            }
        }
    }

    /**
     * DESCENDANT_OF predicate - matches all descendants of a path
     */
    public function descendantOf(string $path): self
    {
        $this->wheres[] = "DESCENDANT_OF('{$path}')";
        return $this;
    }

    /**
     * CHILD_OF predicate - matches direct children of a path
     */
    public function childOf(string $path): self
    {
        $this->wheres[] = "CHILD_OF('{$path}')";
        return $this;
    }

    /**
     * REFERENCES predicate - find nodes that reference a target path
     * Format: workspace:/path
     */
    public function references(string $workspacePath): self
    {
        $this->wheres[] = "REFERENCES('{$workspacePath}')";
        return $this;
    }

    /**
     * Filter by node_type
     */
    public function whereNodeType(string $nodeType): self
    {
        $this->wheres[] = "node_type = ?";
        $this->bindings[] = $nodeType;
        return $this;
    }

    /**
     * Filter by exact path
     */
    public function wherePath(string $path): self
    {
        $this->wheres[] = "path = ?";
        $this->bindings[] = $path;
        return $this;
    }

    /**
     * Filter by node ID
     */
    public function whereId(string $id): self
    {
        $this->wheres[] = "id = ?";
        $this->bindings[] = $id;
        return $this;
    }

    /**
     * Filter by property value using ->> operator
     */
    public function wherePropertyEquals(string $key, mixed $value): self
    {
        $this->wheres[] = "properties ->> '{$key}'::TEXT = ?";
        $this->bindings[] = $value;
        return $this;
    }

    /**
     * Filter by property not equal
     */
    public function wherePropertyNotEquals(string $key, mixed $value): self
    {
        $this->wheres[] = "properties ->> '{$key}'::TEXT != ?";
        $this->bindings[] = $value;
        return $this;
    }

    /**
     * Filter using JSONB containment (@>)
     */
    public function wherePropertyContains(array $conditions): self
    {
        $json = json_encode($conditions);
        $this->wheres[] = "properties @> ?::JSONB";
        $this->bindings[] = $json;
        return $this;
    }

    /**
     * Filter for published articles (status = 'published' AND publishing_date <= NOW())
     */
    public function wherePublished(): self
    {
        $this->wheres[] = "properties ->> 'status'::TEXT = 'published'";
        $this->wheres[] = "(properties ->> 'publishing_date')::TIMESTAMP <= NOW()";
        return $this;
    }

    /**
     * Add a raw WHERE clause
     */
    public function whereRaw(string $clause, array $bindings = []): self
    {
        $this->wheres[] = $clause;
        $this->bindings = array_merge($this->bindings, $bindings);
        return $this;
    }

    /**
     * Add ILIKE search across multiple fields
     */
    public function whereSearchLike(string $query, array $fields): self
    {
        $clauses = [];
        foreach ($fields as $field) {
            if (str_starts_with($field, 'properties.')) {
                $prop = substr($field, 11);
                $clauses[] = "properties ->> '{$prop}'::TEXT ILIKE ?";
            } else {
                $clauses[] = "{$field} ILIKE ?";
            }
            $this->bindings[] = '%' . $query . '%';
        }
        $this->wheres[] = '(' . implode(' OR ', $clauses) . ')';
        return $this;
    }

    /**
     * Exclude a specific path
     */
    public function wherePathNot(string $path): self
    {
        $this->wheres[] = "path != ?";
        $this->bindings[] = $path;
        return $this;
    }

    /**
     * Select specific columns
     */
    public function select(array $columns): self
    {
        $this->columns = $columns;
        return $this;
    }

    /**
     * Order by a column
     */
    public function orderBy(string $column, string $direction = 'ASC'): self
    {
        $this->orderBy = "{$column} {$direction}";
        return $this;
    }

    /**
     * Order by a property value
     */
    public function orderByProperty(string $property, string $direction = 'DESC'): self
    {
        $this->orderBy = "properties ->> '{$property}' {$direction}";
        return $this;
    }

    /**
     * Limit results
     */
    public function limit(int $limit): self
    {
        $this->limit = $limit;
        return $this;
    }

    /**
     * Offset results
     */
    public function offset(int $offset): self
    {
        $this->offset = $offset;
        return $this;
    }

    /**
     * Build and execute the query, return all results
     */
    public function get(): array
    {
        $columns = implode(', ', $this->columns);
        $sql = "SELECT {$columns} FROM {$this->workspace}";

        if (!empty($this->wheres)) {
            $sql .= ' WHERE ' . implode(' AND ', $this->wheres);
        }

        if ($this->orderBy) {
            $sql .= " ORDER BY {$this->orderBy}";
        }

        if ($this->limit) {
            $sql .= " LIMIT {$this->limit}";
        }

        if ($this->offset) {
            $sql .= " OFFSET {$this->offset}";
        }

        $bindings = $this->bindings;

        // Execute with identity context if set
        $results = self::withUserContext(function () use ($sql, $bindings) {
            return DB::select($sql, $bindings);
        });

        // Decode JSON properties
        return array_map(function ($row) {
            if (isset($row->properties) && is_string($row->properties)) {
                $row->properties = json_decode($row->properties);
            }
            return $row;
        }, $results);
    }

    /**
     * Get the first result or null
     */
    public function first(): ?object
    {
        $this->limit = 1;
        $results = $this->get();
        return $results[0] ?? null;
    }

    /**
     * Count matching rows
     */
    public function count(): int
    {
        $sql = "SELECT COUNT(*) as count FROM {$this->workspace}";

        if (!empty($this->wheres)) {
            $sql .= ' WHERE ' . implode(' AND ', $this->wheres);
        }

        $bindings = $this->bindings;

        $result = self::withUserContext(function () use ($sql, $bindings) {
            return DB::selectOne($sql, $bindings);
        });

        return (int) $result->count;
    }

    /**
     * Check if any rows exist
     */
    public function exists(): bool
    {
        return $this->count() > 0;
    }

    // ==================== Static DML Methods ====================

    /**
     * Insert a new node
     */
    public static function insert(string $workspace, array $data): bool
    {
        $sql = "INSERT INTO {$workspace} (path, node_type, name, properties) VALUES (?, ?, ?, ?::JSONB)";
        $bindings = [
            $data['path'],
            $data['node_type'],
            $data['name'],
            json_encode($data['properties'] ?? new \stdClass())
        ];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::statement($sql, $bindings);
        });
    }

    /**
     * Update node properties (merge with existing)
     */
    public static function update(string $workspace, string $path, array $properties): int
    {
        $sql = "UPDATE {$workspace} SET properties = properties || ?::JSONB WHERE path = ?";
        $bindings = [json_encode($properties), $path];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }

    /**
     * Replace all properties
     */
    public static function setProperties(string $workspace, string $path, array $properties): int
    {
        $sql = "UPDATE {$workspace} SET properties = ?::JSONB WHERE path = ?";
        $bindings = [json_encode($properties), $path];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }

    /**
     * Update a specific property using jsonb_set
     */
    public static function setProperty(string $workspace, string $path, string $key, mixed $value): int
    {
        $jsonValue = json_encode($value);
        $sql = "UPDATE {$workspace} SET properties = jsonb_set(properties, ?::TEXT[], ?::JSONB) WHERE path = ?";
        $bindings = ['{' . $key . '}', $jsonValue, $path];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }

    /**
     * Increment a numeric property
     */
    public static function incrementProperty(string $workspace, string $path, string $key, int $amount = 1): int
    {
        $sql = "UPDATE {$workspace} SET properties = jsonb_set(
            properties,
            '{" . $key . "}',
            to_jsonb(COALESCE((properties ->> '" . $key . "')::int, 0) + ?::int)
        ) WHERE path = ?";
        $bindings = [$amount, $path];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }

    /**
     * Delete a node by path
     */
    public static function delete(string $workspace, string $path): int
    {
        $sql = "DELETE FROM {$workspace} WHERE path = ?";
        $bindings = [$path];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }

    /**
     * Delete a node by ID
     */
    public static function deleteById(string $workspace, string $id): int
    {
        $sql = "DELETE FROM {$workspace} WHERE id = ?";
        $bindings = [$id];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }

    /**
     * Move a node to a new parent
     */
    public static function move(string $workspace, string $sourcePath, string $targetParentPath): int
    {
        $name = basename($sourcePath);
        $newPath = rtrim($targetParentPath, '/') . '/' . $name;
        $sql = "MOVE {$workspace} SET path = ? TO path = ?";
        $bindings = [$sourcePath, $newPath];

        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }

    /**
     * Execute raw SQL query
     */
    public static function raw(string $sql, array $bindings = []): array
    {
        $results = self::withUserContext(function () use ($sql, $bindings) {
            return DB::select($sql, $bindings);
        });

        return array_map(function ($row) {
            if (isset($row->properties) && is_string($row->properties)) {
                $row->properties = json_decode($row->properties);
            }
            return $row;
        }, $results);
    }

    /**
     * Execute raw SQL statement (INSERT, UPDATE, DELETE)
     */
    public static function execute(string $sql, array $bindings = []): int
    {
        return self::withUserContext(function () use ($sql, $bindings) {
            return DB::affectingStatement($sql, $bindings);
        });
    }
}
