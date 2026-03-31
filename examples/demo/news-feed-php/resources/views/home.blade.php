@extends('layouts.app')

@section('title', 'Home')

@section('content')
    @php
        $tagMap = [];
        $flatTags = [];
        $addToFlat = function($tags) use (&$flatTags, &$addToFlat) {
            foreach ($tags as $tag) {
                $flatTags[$tag->path] = $tag;
                if (!empty($tag->children)) {
                    $addToFlat($tag->children);
                }
            }
        };
        $addToFlat($tags ?? []);
        $tagMap = $flatTags;
    @endphp

    {{-- Featured Articles --}}
    @if(count($featured) > 0)
        <section class="mb-12">
            <div class="mb-6 flex items-center justify-between">
                <h2 class="text-2xl font-bold text-gray-900">Featured</h2>
            </div>
            <div class="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
                @foreach($featured as $article)
                    @include('components.article-card', [
                        'article' => $article,
                        'categories' => $categories,
                        'tagMap' => $tagMap,
                        'featured' => true
                    ])
                @endforeach
            </div>
        </section>
    @endif

    {{-- Recent Articles --}}
    <section>
        <div class="mb-6 flex items-center justify-between">
            <h2 class="text-2xl font-bold text-gray-900">Recent Articles</h2>
        </div>

        @if(count($recent) > 0)
            <div class="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
                @foreach($recent as $article)
                    @include('components.article-card', [
                        'article' => $article,
                        'categories' => $categories,
                        'tagMap' => $tagMap
                    ])
                @endforeach
            </div>
        @else
            <div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
                <x-lucide-file-text class="mx-auto h-12 w-12 text-gray-400" />
                <h3 class="mt-4 text-lg font-medium text-gray-900">No articles yet</h3>
                <p class="mt-2 text-gray-500">Get started by creating your first article.</p>
                <a href="{{ route('articles.create') }}"
                   class="mt-4 inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700">
                    <x-lucide-plus class="h-4 w-4" />
                    Create Article
                </a>
            </div>
        @endif
    </section>
@endsection
