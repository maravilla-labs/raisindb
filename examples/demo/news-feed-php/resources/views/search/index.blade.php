@extends('layouts.app')

@section('title', $query ? "Search: {$query}" : ($tagPath ? "Tag: {$tagPath}" : 'Search'))

@section('content')
    {{-- Search Header --}}
    <div class="mb-8">
        @if($query)
            <div class="flex items-center gap-3">
                <h1 class="text-2xl font-bold text-gray-900">Search results for "{{ $query }}"</h1>
                <a href="{{ route('search') }}"
                   class="inline-flex items-center gap-1 rounded-full bg-gray-200 px-3 py-1 text-sm text-gray-600 hover:bg-gray-300">
                    <x-lucide-x class="h-3 w-3" />
                    Clear
                </a>
            </div>
            <p class="mt-2 text-gray-500">{{ count($articles) }} article(s) found</p>
        @elseif($tagPath)
            @php
                $tagData = $tagMap['/superbigshit/tags/' . $tagPath] ?? null;
                $tagLabel = $tagData->properties->label ?? basename($tagPath);
                $tagColor = $tagData->properties->color ?? '#6B7280';
            @endphp
            <div class="flex items-center gap-3">
                <h1 class="text-2xl font-bold text-gray-900">Articles tagged with</h1>
                <span class="inline-flex items-center rounded-full px-3 py-1 text-sm font-medium"
                      style="background-color: {{ $tagColor }}20; color: {{ $tagColor }}">
                    {{ $tagLabel }}
                </span>
                <a href="{{ route('search') }}"
                   class="inline-flex items-center gap-1 rounded-full bg-gray-200 px-3 py-1 text-sm text-gray-600 hover:bg-gray-300">
                    <x-lucide-x class="h-3 w-3" />
                    Clear
                </a>
            </div>
            <p class="mt-2 text-gray-500">{{ count($articles) }} article(s) found</p>
        @else
            <h1 class="text-2xl font-bold text-gray-900">Search</h1>
            <p class="mt-2 text-gray-500">Find articles by keyword or browse by tag</p>
        @endif
    </div>

    {{-- Search Form (larger version) --}}
    @if(!$query && !$tagPath)
        <div class="mb-8">
            <form action="{{ route('search') }}" method="GET" class="relative max-w-xl">
                <x-lucide-search class="absolute left-4 top-1/2 h-5 w-5 -translate-y-1/2 text-gray-400" />
                <input type="text"
                       name="q"
                       placeholder="Search for articles..."
                       autofocus
                       class="w-full rounded-xl border border-gray-300 bg-white py-3 pl-12 pr-4 text-lg focus:border-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-500">
            </form>
        </div>
    @endif

    {{-- Results --}}
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
    @elseif($query || $tagPath)
        <div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
            <x-lucide-search-x class="mx-auto h-12 w-12 text-gray-400" />
            <h3 class="mt-4 text-lg font-medium text-gray-900">No results found</h3>
            <p class="mt-2 text-gray-500">Try adjusting your search or browse different tags.</p>
        </div>
    @endif
@endsection
