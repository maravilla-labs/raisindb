<?php

namespace App\Services\RaisinDB;

class TagService
{
    protected const WORKSPACE = 'social';
    protected const TAGS_PATH = '/superbigshit/tags';

    /**
     * Get all tags as hierarchical tree
     */
    public function getAll(): array
    {
        $tags = RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::TAGS_PATH)
            ->whereNodeType('news:Tag')
            ->orderBy('path', 'ASC')
            ->get();

        return $this->buildTree($tags);
    }

    /**
     * Get all tags as flat list
     */
    public function getAllFlat(): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::TAGS_PATH)
            ->whereNodeType('news:Tag')
            ->orderBy('path', 'ASC')
            ->get();
    }

    /**
     * Get tag by path
     */
    public function getByPath(string $path): ?object
    {
        $fullPath = self::TAGS_PATH . '/' . ltrim($path, '/');

        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->wherePath($fullPath)
            ->whereNodeType('news:Tag')
            ->first();
    }

    /**
     * Create a new tag
     */
    public function create(array $data): string
    {
        $parentPath = $data['parent_path'] ?? self::TAGS_PATH;
        if (!str_starts_with($parentPath, self::TAGS_PATH)) {
            $parentPath = self::TAGS_PATH . '/' . ltrim($parentPath, '/');
        }

        $slug = $this->slugify($data['label']);
        $path = rtrim($parentPath, '/') . '/' . $slug;

        $properties = [
            'label' => $data['label'],
            'icon' => $data['icon'] ?? null,
            'color' => $data['color'] ?? '#6B7280',
        ];

        RaisinQueryBuilder::insert(self::WORKSPACE, [
            'path' => $path,
            'node_type' => 'news:Tag',
            'name' => $data['label'],
            'properties' => $properties,
        ]);

        return $path;
    }

    /**
     * Update an existing tag
     */
    public function update(string $path, array $data): void
    {
        $fullPath = self::TAGS_PATH . '/' . ltrim($path, '/');

        $properties = [
            'label' => $data['label'],
            'icon' => $data['icon'] ?? null,
            'color' => $data['color'] ?? '#6B7280',
        ];

        RaisinQueryBuilder::setProperties(self::WORKSPACE, $fullPath, $properties);
    }

    /**
     * Delete a tag
     */
    public function delete(string $path): bool
    {
        $fullPath = self::TAGS_PATH . '/' . ltrim($path, '/');
        return RaisinQueryBuilder::delete(self::WORKSPACE, $fullPath) > 0;
    }

    /**
     * Build hierarchical tree from flat list
     */
    protected function buildTree(array $tags): array
    {
        $tree = [];
        $lookup = [];

        foreach ($tags as $tag) {
            $tag->children = [];
            $lookup[$tag->path] = $tag;
        }

        foreach ($tags as $tag) {
            $parentPath = dirname($tag->path);
            if (isset($lookup[$parentPath])) {
                $lookup[$parentPath]->children[] = $tag;
            } else {
                $tree[] = $tag;
            }
        }

        return $tree;
    }

    /**
     * Generate URL-friendly slug
     */
    protected function slugify(string $text): string
    {
        $text = preg_replace('/[^a-zA-Z0-9\s-]/', '', $text);
        $text = preg_replace('/[\s_-]+/', '-', $text);
        $text = trim($text, '-');
        return strtolower($text);
    }
}
