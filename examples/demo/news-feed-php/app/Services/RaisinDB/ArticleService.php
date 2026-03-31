<?php

namespace App\Services\RaisinDB;

class ArticleService
{
    protected const WORKSPACE = 'social';
    protected const BASE_PATH = '/superbigshit';
    protected const ARTICLES_PATH = '/superbigshit/articles';
    protected const TAGS_PATH = '/superbigshit/tags';

    /**
     * Get featured articles
     */
    public function getFeatured(int $limit = 3): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH)
            ->whereNodeType('news:Article')
            ->wherePropertyContains(['featured' => true, 'status' => 'published'])
            ->wherePublished()
            ->orderByProperty('publishing_date', 'DESC')
            ->limit($limit)
            ->get();
    }

    /**
     * Get recent published articles
     */
    public function getRecent(int $limit = 12): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH)
            ->whereNodeType('news:Article')
            ->wherePublished()
            ->orderByProperty('publishing_date', 'DESC')
            ->limit($limit)
            ->get();
    }

    /**
     * Get all articles (including drafts)
     */
    public function getAll(int $limit = 100): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH)
            ->whereNodeType('news:Article')
            ->orderByProperty('publishing_date', 'DESC')
            ->limit($limit)
            ->get();
    }

    /**
     * Get all articles except one (for connection picker)
     */
    public function getAllExcept(string $excludePath): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH)
            ->whereNodeType('news:Article')
            ->wherePathNot(self::ARTICLES_PATH . '/' . ltrim($excludePath, '/'))
            ->orderByProperty('title', 'ASC')
            ->limit(100)
            ->get();
    }

    /**
     * Get article by path
     */
    public function getByPath(string $path): ?object
    {
        $fullPath = self::ARTICLES_PATH . '/' . ltrim($path, '/');

        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->wherePath($fullPath)
            ->whereNodeType('news:Article')
            ->first();
    }

    /**
     * Get articles by category slug
     */
    public function getByCategory(string $categorySlug, int $limit = 50): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH . '/' . $categorySlug)
            ->whereNodeType('news:Article')
            ->wherePublished()
            ->orderByProperty('publishing_date', 'DESC')
            ->limit($limit)
            ->get();
    }

    /**
     * Get related articles in the same category
     */
    public function getRelatedByCategory(string $articlePath, int $limit = 5): array
    {
        $article = $this->getByPath($articlePath);
        if (!$article) {
            return [];
        }

        $category = $article->properties->category ?? null;
        if (!$category) {
            return [];
        }

        $fullPath = self::ARTICLES_PATH . '/' . ltrim($articlePath, '/');

        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH . '/' . $category)
            ->whereNodeType('news:Article')
            ->wherePublished()
            ->wherePathNot($fullPath)
            ->orderByProperty('publishing_date', 'DESC')
            ->limit($limit)
            ->get();
    }

    /**
     * Search articles by keyword
     */
    public function searchByKeyword(string $query, int $limit = 20): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::ARTICLES_PATH)
            ->whereNodeType('news:Article')
            ->wherePublished()
            ->whereSearchLike($query, [
                'properties.title',
                'properties.body',
                'properties.excerpt',
                'properties.keywords'
            ])
            ->orderByProperty('publishing_date', 'DESC')
            ->limit($limit)
            ->get();
    }

    /**
     * Search articles by tag reference
     */
    public function searchByTag(string $tagPath, int $limit = 20): array
    {
        $fullTagPath = self::TAGS_PATH . '/' . ltrim($tagPath, '/');
        $reference = self::WORKSPACE . ':' . $fullTagPath;

        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->references($reference)
            ->whereNodeType('news:Article')
            ->wherePublished()
            ->orderByProperty('publishing_date', 'DESC')
            ->limit($limit)
            ->get();
    }

    /**
     * Get all categories
     */
    public function getCategories(): array
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
    public function getCategoryBySlug(string $slug): ?object
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->wherePath(self::ARTICLES_PATH . '/' . $slug)
            ->whereNodeType('raisin:Folder')
            ->first();
    }

    /**
     * Get category from article path
     */
    public function getCategoryFromPath(string $articlePath): ?string
    {
        $parts = explode('/', trim($articlePath, '/'));
        return $parts[0] ?? null;
    }

    /**
     * Get all tags as hierarchical tree
     */
    public function getTags(): array
    {
        $tags = RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::TAGS_PATH)
            ->whereNodeType('news:Tag')
            ->orderBy('path', 'ASC')
            ->get();

        return $this->buildTagTree($tags);
    }

    /**
     * Get flat list of all tags
     */
    public function getTagsFlat(): array
    {
        return RaisinQueryBuilder::query(self::WORKSPACE)
            ->descendantOf(self::TAGS_PATH)
            ->whereNodeType('news:Tag')
            ->orderBy('path', 'ASC')
            ->get();
    }

    /**
     * Build hierarchical tag tree from flat list
     */
    protected function buildTagTree(array $tags): array
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
     * Increment article view count
     */
    public function incrementViews(string $articlePath): void
    {
        $fullPath = self::ARTICLES_PATH . '/' . ltrim($articlePath, '/');
        RaisinQueryBuilder::incrementProperty(self::WORKSPACE, $fullPath, 'views', 1);
    }

    /**
     * Get graph data for article sidebar
     */
    public function getGraphData(string $articlePath): array
    {
        $fullPath = self::ARTICLES_PATH . '/' . ltrim($articlePath, '/');

        return [
            'correction' => GraphQueryBuilder::findCorrection($fullPath),
            'timeline' => GraphQueryBuilder::findStoryTimeline($fullPath),
            'contradictions' => GraphQueryBuilder::findContradictions($fullPath),
            'evidence' => GraphQueryBuilder::findEvidenceSources($fullPath),
            'related' => GraphQueryBuilder::findSmartRelated($fullPath),
        ];
    }

    /**
     * Get incoming connections for an article
     */
    public function getIncomingConnections(string $articlePath): array
    {
        $fullPath = self::ARTICLES_PATH . '/' . ltrim($articlePath, '/');
        return GraphQueryBuilder::getIncomingConnections($fullPath);
    }

    /**
     * Create a new article
     */
    public function create(array $data): string
    {
        $category = $data['category'];
        $slug = $data['slug'];
        $path = self::ARTICLES_PATH . '/' . $category . '/' . $slug;

        $properties = [
            'title' => $data['title'],
            'slug' => $slug,
            'excerpt' => $data['excerpt'] ?? '',
            'body' => $data['body'] ?? '',
            'keywords' => $data['keywords'] ?? [],
            'tags' => $data['tags'] ?? [],
            'featured' => $data['featured'] ?? false,
            'status' => $data['status'] ?? 'draft',
            'publishing_date' => $data['publishing_date'] ?? now()->toIso8601String(),
            'views' => 0,
            'author' => $data['author'] ?? '',
            'imageUrl' => $data['imageUrl'] ?? '',
            'category' => $category,
            'connections' => $data['connections'] ?? [],
        ];

        RaisinQueryBuilder::insert(self::WORKSPACE, [
            'path' => $path,
            'node_type' => 'news:Article',
            'name' => $data['title'],
            'properties' => $properties,
        ]);

        // Create graph relations for connections
        $this->syncConnections($path, $data['connections'] ?? []);

        return $path;
    }

    /**
     * Update an existing article
     */
    public function update(string $path, array $data): string
    {
        $fullPath = self::ARTICLES_PATH . '/' . ltrim($path, '/');
        $currentArticle = $this->getByPath($path);

        if (!$currentArticle) {
            throw new \Exception("Article not found: {$path}");
        }

        // Check if category changed (need to move)
        $currentCategory = $this->getCategoryFromPath($path);
        $newCategory = $data['category'] ?? $currentCategory;

        if ($currentCategory !== $newCategory) {
            // Move to new category
            $newPath = self::ARTICLES_PATH . '/' . $newCategory;
            RaisinQueryBuilder::move(self::WORKSPACE, $fullPath, $newPath);
            $fullPath = $newPath . '/' . basename($fullPath);
        }

        $properties = [
            'title' => $data['title'],
            'slug' => $data['slug'],
            'excerpt' => $data['excerpt'] ?? '',
            'body' => $data['body'] ?? '',
            'keywords' => $data['keywords'] ?? [],
            'tags' => $data['tags'] ?? [],
            'featured' => $data['featured'] ?? false,
            'status' => $data['status'] ?? 'draft',
            'publishing_date' => $data['publishing_date'] ?? $currentArticle->properties->publishing_date,
            'author' => $data['author'] ?? '',
            'imageUrl' => $data['imageUrl'] ?? '',
            'category' => $newCategory,
            'connections' => $data['connections'] ?? [],
        ];

        RaisinQueryBuilder::setProperties(self::WORKSPACE, $fullPath, $properties);

        // Sync graph relations
        $this->syncConnections($fullPath, $data['connections'] ?? []);

        return str_replace(self::ARTICLES_PATH . '/', '', $fullPath);
    }

    /**
     * Delete an article
     */
    public function delete(string $path): bool
    {
        $fullPath = self::ARTICLES_PATH . '/' . ltrim($path, '/');
        return RaisinQueryBuilder::delete(self::WORKSPACE, $fullPath) > 0;
    }

    /**
     * Move article to a different category
     */
    public function moveToCategory(string $articlePath, string $newCategory): string
    {
        $fullPath = self::ARTICLES_PATH . '/' . ltrim($articlePath, '/');
        $newParentPath = self::ARTICLES_PATH . '/' . $newCategory;
        $slug = basename($fullPath);

        RaisinQueryBuilder::move(self::WORKSPACE, $fullPath, $newParentPath);

        // Update category in properties
        $newFullPath = $newParentPath . '/' . $slug;
        RaisinQueryBuilder::update(self::WORKSPACE, $newFullPath, ['category' => $newCategory]);

        return $newCategory . '/' . $slug;
    }

    /**
     * Sync graph connections for an article
     */
    protected function syncConnections(string $articlePath, array $connections): void
    {
        // Get current outgoing connections
        $workspacePath = self::WORKSPACE . ':' . $articlePath;
        $currentConnections = GraphQueryBuilder::neighbors($workspacePath, 'OUT', null);

        // Remove old connections
        foreach ($currentConnections as $conn) {
            GraphQueryBuilder::unrelate($articlePath, $conn->path, $conn->relation_type, self::WORKSPACE);
        }

        // Create new connections
        foreach ($connections as $conn) {
            $targetPath = $conn['targetPath'] ?? null;
            $relationType = $conn['relationType'] ?? 'similar-to';
            $weight = ($conn['weight'] ?? 75) / 100; // Convert from 0-100 to 0-1

            if ($targetPath) {
                GraphQueryBuilder::relate($articlePath, $targetPath, $relationType, $weight, self::WORKSPACE);
            }
        }
    }
}
