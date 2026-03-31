<?php

namespace App\Http\Controllers;

use App\Services\RaisinDB\ArticleService;
use Illuminate\Http\Request;

class ArticleController extends Controller
{
    public function __construct(
        protected ArticleService $articleService
    ) {}

    /**
     * Home page - show featured and recent articles
     */
    public function index()
    {
        $featured = $this->articleService->getFeatured(3);
        $recent = $this->articleService->getRecent(12);
        $categories = $this->articleService->getCategories();
        $tags = $this->articleService->getTags();

        return view('home', compact('featured', 'recent', 'categories', 'tags'));
    }

    /**
     * Show a category page or article page based on path
     */
    public function show(string $path)
    {
        $segments = explode('/', trim($path, '/'));
        $categories = $this->articleService->getCategories();
        $tags = $this->articleService->getTags();

        // Single segment = category page
        if (count($segments) === 1) {
            $category = $this->articleService->getCategoryBySlug($segments[0]);

            if (!$category) {
                abort(404, 'Category not found');
            }

            $articles = $this->articleService->getByCategory($segments[0]);

            return view('categories.show', compact('category', 'articles', 'categories', 'tags'));
        }

        // Multiple segments = article page
        $article = $this->articleService->getByPath($path);

        if (!$article) {
            abort(404, 'Article not found');
        }

        // Increment view count
        $this->articleService->incrementViews($path);

        // Get graph data for sidebar widgets
        $graphData = $this->articleService->getGraphData($path);
        $related = $this->articleService->getRelatedByCategory($path, 3);
        $category = $this->articleService->getCategoryBySlug($segments[0]);

        // Build tag map for easy lookup
        $tagMap = $this->buildTagMap($this->articleService->getTagsFlat());

        return view('articles.show', compact(
            'article', 'graphData', 'related', 'categories', 'tags', 'category', 'tagMap'
        ));
    }

    /**
     * Show create article form
     */
    public function create()
    {
        $categories = $this->articleService->getCategories();
        $tags = $this->articleService->getTags();

        return view('articles.create', compact('categories', 'tags'));
    }

    /**
     * Store a new article
     */
    public function store(Request $request)
    {
        $validated = $request->validate([
            'title' => 'required|string|max:255',
            'slug' => 'required|string|max:255',
            'category' => 'required|string',
            'excerpt' => 'nullable|string',
            'body' => 'nullable|string',
            'author' => 'nullable|string|max:255',
            'imageUrl' => 'nullable|url',
            'publishing_date' => 'nullable|date',
            'featured' => 'nullable|boolean',
            'status' => 'required|in:draft,published',
            'tags' => 'nullable|string',
            'connections' => 'nullable|string',
            'keywords' => 'nullable|string',
        ]);

        // Parse JSON fields
        $validated['tags'] = json_decode($validated['tags'] ?? '[]', true) ?: [];
        $validated['connections'] = json_decode($validated['connections'] ?? '[]', true) ?: [];
        $validated['keywords'] = array_filter(array_map('trim', explode(',', $validated['keywords'] ?? '')));
        $validated['featured'] = (bool) ($validated['featured'] ?? false);

        $path = $this->articleService->create($validated);
        $articlePath = str_replace('/superbigshit/articles/', '', $path);

        return redirect()->route('articles.show', ['path' => $articlePath])
            ->with('success', 'Article created successfully');
    }

    /**
     * Show edit article form
     */
    public function edit(string $path)
    {
        $article = $this->articleService->getByPath($path);

        if (!$article) {
            abort(404, 'Article not found');
        }

        $categories = $this->articleService->getCategories();
        $tags = $this->articleService->getTags();
        $availableArticles = $this->articleService->getAllExcept($path);
        $incomingConnections = $this->articleService->getIncomingConnections($path);
        $currentCategory = $this->articleService->getCategoryFromPath($path);

        return view('articles.edit', compact(
            'article', 'categories', 'tags', 'availableArticles',
            'incomingConnections', 'currentCategory'
        ));
    }

    /**
     * Update an article
     */
    public function update(Request $request, string $path)
    {
        $validated = $request->validate([
            'title' => 'required|string|max:255',
            'slug' => 'required|string|max:255',
            'category' => 'required|string',
            'excerpt' => 'nullable|string',
            'body' => 'nullable|string',
            'author' => 'nullable|string|max:255',
            'imageUrl' => 'nullable|url',
            'publishing_date' => 'nullable|date',
            'featured' => 'nullable|boolean',
            'status' => 'required|in:draft,published',
            'tags' => 'nullable|string',
            'connections' => 'nullable|string',
            'keywords' => 'nullable|string',
        ]);

        // Parse JSON fields
        $validated['tags'] = json_decode($validated['tags'] ?? '[]', true) ?: [];
        $validated['connections'] = json_decode($validated['connections'] ?? '[]', true) ?: [];
        $validated['keywords'] = array_filter(array_map('trim', explode(',', $validated['keywords'] ?? '')));
        $validated['featured'] = (bool) ($validated['featured'] ?? false);

        $newPath = $this->articleService->update($path, $validated);

        return redirect()->route('articles.show', ['path' => $newPath])
            ->with('success', 'Article updated successfully');
    }

    /**
     * Delete an article
     */
    public function destroy(string $path)
    {
        $this->articleService->delete($path);

        return redirect()->route('home')
            ->with('success', 'Article deleted successfully');
    }

    /**
     * Show move article form
     */
    public function showMove(string $path)
    {
        $article = $this->articleService->getByPath($path);

        if (!$article) {
            abort(404, 'Article not found');
        }

        $categories = $this->articleService->getCategories();
        $currentCategory = $this->articleService->getCategoryFromPath($path);

        return view('articles.move', compact('article', 'categories', 'currentCategory'));
    }

    /**
     * Move article to different category
     */
    public function move(Request $request, string $path)
    {
        $request->validate(['category' => 'required|string']);

        $newPath = $this->articleService->moveToCategory($path, $request->category);

        return redirect()->route('articles.show', ['path' => $newPath])
            ->with('success', 'Article moved successfully');
    }

    /**
     * Build a map of tag paths to tag data
     */
    protected function buildTagMap(array $tags): array
    {
        $map = [];
        foreach ($tags as $tag) {
            $map[$tag->path] = $tag;
        }
        return $map;
    }
}
