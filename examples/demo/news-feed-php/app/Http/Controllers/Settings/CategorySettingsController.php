<?php

namespace App\Http\Controllers\Settings;

use App\Http\Controllers\Controller;
use App\Services\RaisinDB\ArticleService;
use App\Services\RaisinDB\CategoryService;
use Illuminate\Http\Request;

class CategorySettingsController extends Controller
{
    public function __construct(
        protected CategoryService $categoryService,
        protected ArticleService $articleService
    ) {}

    /**
     * List all categories
     */
    public function index()
    {
        $settingsCategories = $this->categoryService->getAll();
        $categories = $this->articleService->getCategories();
        $tags = $this->articleService->getTags();

        // Add article counts
        foreach ($settingsCategories as $category) {
            $slug = basename($category->path);
            $category->article_count = $this->categoryService->getArticleCount($slug);
        }

        return view('settings.categories.index', compact('settingsCategories', 'categories', 'tags'));
    }

    /**
     * Create a new category
     */
    public function store(Request $request)
    {
        $validated = $request->validate([
            'label' => 'required|string|max:255',
            'color' => 'nullable|string|max:7',
            'description' => 'nullable|string',
        ]);

        $this->categoryService->create($validated);

        return redirect()->route('settings.categories.index')
            ->with('success', 'Category created successfully');
    }

    /**
     * Update a category
     */
    public function update(Request $request, string $slug)
    {
        $validated = $request->validate([
            'label' => 'required|string|max:255',
            'color' => 'nullable|string|max:7',
            'description' => 'nullable|string',
        ]);

        $this->categoryService->update($slug, $validated);

        return redirect()->route('settings.categories.index')
            ->with('success', 'Category updated successfully');
    }

    /**
     * Delete a category
     */
    public function destroy(string $slug)
    {
        // Check if category has articles
        $count = $this->categoryService->getArticleCount($slug);

        if ($count > 0) {
            return redirect()->route('settings.categories.index')
                ->with('error', "Cannot delete category with {$count} articles. Move or delete the articles first.");
        }

        $this->categoryService->delete($slug);

        return redirect()->route('settings.categories.index')
            ->with('success', 'Category deleted successfully');
    }
}
