@extends('layouts.app')

@section('title', $category->properties->label ?? $category->name)

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
        $color = $category->properties->color ?? '#6B7280';
    @endphp

    {{-- Category Header --}}
    <div class="mb-8">
        <div class="flex items-center gap-3">
            <span class="inline-flex h-10 w-10 items-center justify-center rounded-lg"
                  style="background-color: {{ $color }}20">
                <x-lucide-folder class="h-5 w-5" style="color: {{ $color }}" />
            </span>
            <div>
                <h1 class="text-2xl font-bold text-gray-900">{{ $category->properties->label ?? $category->name }}</h1>
                @if($category->properties->description ?? false)
                    <p class="text-gray-500">{{ $category->properties->description }}</p>
                @endif
            </div>
        </div>
    </div>

    {{-- Articles Grid --}}
    @if(count($articles) > 0)
        <div class="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
            @foreach($articles as $article)
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
            <h3 class="mt-4 text-lg font-medium text-gray-900">No articles in this category</h3>
            <p class="mt-2 text-gray-500">Create an article to add it to this category.</p>
            <a href="{{ route('articles.create') }}"
               class="mt-4 inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700">
                <x-lucide-plus class="h-4 w-4" />
                Create Article
            </a>
        </div>
    @endif
@endsection
