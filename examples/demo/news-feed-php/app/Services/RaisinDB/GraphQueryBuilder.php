<?php

namespace App\Services\RaisinDB;

use Illuminate\Support\Facades\DB;

class GraphQueryBuilder
{
    protected string $matchPattern = '';
    protected array $wheres = [];
    protected array $columns = [];
    protected ?string $orderBy = null;
    protected ?int $limit = null;

    /**
     * Set the MATCH pattern for graph traversal
     *
     * Examples:
     * - (a:Article)-[r:`similar-to`]->(b:Article)
     * - (this:Article)<-[:corrects]-(correction:Article)
     * - (start:Article)-[:`similar-to`*2]->(distant:Article)
     */
    public function match(string $pattern): self
    {
        $this->matchPattern = $pattern;
        return $this;
    }

    /**
     * Add a WHERE condition
     */
    public function where(string $condition): self
    {
        $this->wheres[] = $condition;
        return $this;
    }

    /**
     * Set the COLUMNS to return
     *
     * Examples:
     * - ['b.id AS related_id', 'b.path AS related_path', 'r.weight AS score']
     */
    public function columns(array $columns): self
    {
        $this->columns = $columns;
        return $this;
    }

    /**
     * Order by a column (prefixed with g. for the alias)
     */
    public function orderBy(string $column, string $direction = 'ASC'): self
    {
        $this->orderBy = "g.{$column} {$direction}";
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
     * Execute the GRAPH_TABLE query and return results
     */
    public function get(): array
    {
        $columnsStr = implode(', ', $this->columns);
        $whereStr = !empty($this->wheres)
            ? 'WHERE ' . implode(' AND ', $this->wheres)
            : '';

        $sql = "SELECT * FROM GRAPH_TABLE(
            MATCH {$this->matchPattern}
            {$whereStr}
            COLUMNS ({$columnsStr})
        ) AS g";

        if ($this->orderBy) {
            $sql .= " ORDER BY {$this->orderBy}";
        }

        if ($this->limit) {
            $sql .= " LIMIT {$this->limit}";
        }

        return DB::select($sql);
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

    // ==================== Static Helper Methods ====================

    /**
     * Query neighbors using the NEIGHBORS() table-valued function
     *
     * @param string $startNode Format: 'workspace:/path' or node ID
     * @param string $direction 'OUT', 'IN', or 'BOTH'
     * @param string|null $relationType Filter by relation type, or null for all
     */
    public static function neighbors(
        string $startNode,
        string $direction = 'OUT',
        ?string $relationType = null
    ): array {
        $typeParam = $relationType !== null ? "'{$relationType}'" : 'NULL';
        $sql = "SELECT n.id, n.path, n.name, n.node_type, n.properties, n.relation_type, n.weight
                FROM NEIGHBORS('{$startNode}', '{$direction}', {$typeParam}) AS n";

        $results = DB::select($sql);

        return array_map(function ($row) {
            if (isset($row->properties) && is_string($row->properties)) {
                $row->properties = json_decode($row->properties);
            }
            return $row;
        }, $results);
    }

    /**
     * Create a relationship between two nodes
     *
     * @param string $sourcePath Source node path
     * @param string $targetPath Target node path
     * @param string $relationType The relationship type
     * @param float $weight Relationship weight (0.0-1.0)
     * @param string $workspace Workspace name
     */
    public static function relate(
        string $sourcePath,
        string $targetPath,
        string $relationType,
        float $weight = 0.75,
        string $workspace = 'social'
    ): bool {
        $sql = "RELATE FROM path='{$sourcePath}' IN WORKSPACE '{$workspace}'
                TO path='{$targetPath}' IN WORKSPACE '{$workspace}'
                TYPE '{$relationType}' WEIGHT {$weight}";
        return DB::statement($sql);
    }

    /**
     * Remove a relationship between two nodes
     *
     * @param string $sourcePath Source node path
     * @param string $targetPath Target node path
     * @param string|null $relationType Specific type to remove, or null for all
     * @param string $workspace Workspace name
     */
    public static function unrelate(
        string $sourcePath,
        string $targetPath,
        ?string $relationType = null,
        string $workspace = 'social'
    ): bool {
        $sql = "UNRELATE FROM path='{$sourcePath}' IN WORKSPACE '{$workspace}'
                TO path='{$targetPath}' IN WORKSPACE '{$workspace}'";

        if ($relationType !== null) {
            $sql .= " TYPE '{$relationType}'";
        }

        return DB::statement($sql);
    }

    /**
     * Find articles that correct a given article
     */
    public static function findCorrection(string $articlePath): ?object
    {
        $escapedPath = addslashes($articlePath);

        $builder = new self();
        $result = $builder
            ->match('(this:Article)<-[:`corrects`]-(correction:Article)')
            ->where("this.path = '{$escapedPath}'")
            ->columns([
                'correction.id AS id',
                'correction.path AS path',
                'correction.name AS name',
                'correction.properties AS properties'
            ])
            ->limit(1)
            ->first();

        if ($result && isset($result->properties) && is_string($result->properties)) {
            $result->properties = json_decode($result->properties);
        }

        return $result;
    }

    /**
     * Find story timeline (predecessors and successors via continues/updates)
     */
    public static function findStoryTimeline(string $articlePath): array
    {
        $escapedPath = addslashes($articlePath);

        // Find predecessors (articles that this one continues/updates)
        $predecessors = (new self())
            ->match('(this:Article)-[r:`continues`|`updates`]->(predecessor:Article)')
            ->where("this.path = '{$escapedPath}'")
            ->columns([
                'predecessor.id AS id',
                'predecessor.path AS path',
                'predecessor.name AS name',
                'predecessor.properties AS properties',
                'r.type AS relation_type'
            ])
            ->get();

        // Find successors (articles that continue/update this one)
        $successors = (new self())
            ->match('(this:Article)<-[r:`continues`|`updates`]-(successor:Article)')
            ->where("this.path = '{$escapedPath}'")
            ->columns([
                'successor.id AS id',
                'successor.path AS path',
                'successor.name AS name',
                'successor.properties AS properties',
                'r.type AS relation_type'
            ])
            ->get();

        // Decode properties
        $decode = function ($items) {
            return array_map(function ($item) {
                if (isset($item->properties) && is_string($item->properties)) {
                    $item->properties = json_decode($item->properties);
                }
                return $item;
            }, $items);
        };

        return [
            'predecessors' => $decode($predecessors),
            'successors' => $decode($successors),
        ];
    }

    /**
     * Find contradicting articles
     */
    public static function findContradictions(string $articlePath, int $limit = 5): array
    {
        $escapedPath = addslashes($articlePath);

        $builder = new self();
        $results = $builder
            ->match('(this:Article)-[r:`contradicts`]-(other:Article)')
            ->where("this.path = '{$escapedPath}'")
            ->columns([
                'other.id AS id',
                'other.path AS path',
                'other.name AS name',
                'other.properties AS properties',
                'r.weight AS weight'
            ])
            ->orderBy('weight', 'DESC')
            ->limit($limit)
            ->get();

        return array_map(function ($row) {
            if (isset($row->properties) && is_string($row->properties)) {
                $row->properties = json_decode($row->properties);
            }
            return $row;
        }, $results);
    }

    /**
     * Find articles that provide evidence for this article
     */
    public static function findEvidenceSources(string $articlePath, int $limit = 5): array
    {
        $escapedPath = addslashes($articlePath);

        $builder = new self();
        $results = $builder
            ->match('(this:Article)<-[r:`provides-evidence-for`]-(source:Article)')
            ->where("this.path = '{$escapedPath}'")
            ->columns([
                'source.id AS id',
                'source.path AS path',
                'source.name AS name',
                'source.properties AS properties',
                'r.weight AS weight'
            ])
            ->orderBy('weight', 'DESC')
            ->limit($limit)
            ->get();

        return array_map(function ($row) {
            if (isset($row->properties) && is_string($row->properties)) {
                $row->properties = json_decode($row->properties);
            }
            return $row;
        }, $results);
    }

    /**
     * Find smart related articles (similar-to, see-also, updates)
     */
    public static function findSmartRelated(string $articlePath, int $limit = 5): array
    {
        $escapedPath = addslashes($articlePath);

        $builder = new self();
        $results = $builder
            ->match('(this:Article)-[r:`similar-to`|`see-also`|`updates`]->(related:Article)')
            ->where("this.path = '{$escapedPath}'")
            ->columns([
                'related.id AS id',
                'related.path AS path',
                'related.name AS name',
                'related.properties AS properties',
                'r.type AS relation_type',
                'r.weight AS weight'
            ])
            ->orderBy('weight', 'DESC')
            ->limit($limit)
            ->get();

        return array_map(function ($row) {
            if (isset($row->properties) && is_string($row->properties)) {
                $row->properties = json_decode($row->properties);
            }
            return $row;
        }, $results);
    }

    /**
     * Get incoming connections to an article
     */
    public static function getIncomingConnections(string $articlePath): array
    {
        $workspace = 'social';
        $workspacePath = "{$workspace}:{$articlePath}";

        return self::neighbors($workspacePath, 'IN', null);
    }
}
