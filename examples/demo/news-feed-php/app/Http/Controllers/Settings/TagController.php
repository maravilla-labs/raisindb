<?php

namespace App\Http\Controllers\Settings;

use App\Http\Controllers\Controller;
use App\Services\RaisinDB\ArticleService;
use App\Services\RaisinDB\TagService;
use Illuminate\Http\Request;

class TagController extends Controller
{
    public function __construct(
        protected TagService $tagService,
        protected ArticleService $articleService
    ) {}

    /**
     * Show tag management page
     */
    public function index()
    {
        $tagTree = $this->tagService->getAll();
        $categories = $this->articleService->getCategories();
        $tags = $this->articleService->getTags();

        return view('settings.tags.index', compact('tagTree', 'categories', 'tags'));
    }

    /**
     * Create a new tag
     */
    public function store(Request $request)
    {
        $validated = $request->validate([
            'label' => 'required|string|max:255',
            'icon' => 'nullable|string|max:50',
            'color' => 'nullable|string|max:7',
            'parent_path' => 'nullable|string',
        ]);

        $this->tagService->create($validated);

        return redirect()->route('settings.tags.index')
            ->with('success', 'Tag created successfully');
    }

    /**
     * Update a tag
     */
    public function update(Request $request, string $path)
    {
        $validated = $request->validate([
            'label' => 'required|string|max:255',
            'icon' => 'nullable|string|max:50',
            'color' => 'nullable|string|max:7',
        ]);

        $this->tagService->update($path, $validated);

        return redirect()->route('settings.tags.index')
            ->with('success', 'Tag updated successfully');
    }

    /**
     * Delete a tag
     */
    public function destroy(string $path)
    {
        $this->tagService->delete($path);

        return redirect()->route('settings.tags.index')
            ->with('success', 'Tag deleted successfully');
    }
}
