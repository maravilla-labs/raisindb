<?php

namespace App\Services\RaisinDB;

class CategoryService
{
    protected const WORKSPACE = 'social';
    protected const ARTICLES_PATH = '/superbigshit/articles';

    /**
     * Get all categories
     */
    public function getAll(): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->childOf(self::ARTICLES_PATH)
            ->whereNodeType('raisin:Folder')
            ->orderBy('path', 'ASC')
            ->get();
    }

    /**
     * Get category by slug
     */
    public function getBySlug(string $slug): ?object
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->wherePath(self::ARTICLES_PATH . '/' . $slug)
            ->whereNodeType('raisin:Folder')
            ->first();
    }

    /**
     * Create a new category
     */
    public function create(array $data): string
    {
        $slug = $this->slugify($data['label']);
        $path = self::ARTICLES_PATH . '/' . $slug;

        $properties = [
            'label' => $data['label'],
            'color' => $data['color'] ?? '#6B7280',
            'description' => $data['description'] ?? '',
        ];

        RaisinQueryBuilder::insert(self::WORKSPACE, [
            'path' => $path,
            'node_type' => 'raisin:Folder',
            'name' => $data['label'],
            'properties' => $properties,
        ]);

        return $path;
    }

    /**
     * Update an existing category
     */
    public function update(string $slug, array $data): void
    {
        $fullPath = self::ARTICLES_PATH . '/' . $slug;

        $properties = [
            'label' => $data['label'],
            'color' => $data['color'] ?? '#6B7280',
            'description' => $data['description'] ?? '',
        ];

        RaisinQueryBuilder::setProperties(self::WORKSPACE, $fullPath, $properties);
    }

    /**
     * Delete a category
     */
    public function delete(string $slug): bool
    {
        $fullPath = self::ARTICLES_PATH . '/' . $slug;
        return RaisinQueryBuilder::delete(self::WORKSPACE, $fullPath) > 0;
    }

    /**
     * Get article count for a category
     */
    public function getArticleCount(string $slug): int
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH . '/' . $slug)
            ->whereNodeType('news:Article')
            ->count();
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
