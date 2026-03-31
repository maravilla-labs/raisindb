<?php

namespace App\Http\Controllers;

use App\Services\RaisinDB\ArticleService;
use Illuminate\Http\Request;

class SearchController extends Controller
{
    public function __construct(
        protected ArticleService $articleService
    ) {}

    /**
     * Search articles by keyword or tag
     */
    public function index(Request $request)
    {
        $query = $request->get('q');
        $tagPath = $request->get('tag');
        $articles = [];

        if ($query) {
            $articles = $this->articleService->searchByKeyword($query);
        } elseif ($tagPath) {
            $articles = $this->articleService->searchByTag($tagPath);
        }

        $categories = $this->articleService->getCategories();
        $tags = $this->articleService->getTags();
        $tagMap = $this->buildTagMap($this->articleService->getTagsFlat());

        return view('search.index', compact('articles', 'query', 'tagPath', 'categories', 'tags', 'tagMap'));
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
