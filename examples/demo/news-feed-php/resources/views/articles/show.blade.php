@extends('layouts.app')

@section('title', $article->properties->title)

@section('content')
    @php
        $articlePath = str_replace('/superbigshit/articles/', '', $article->path);
        $categorySlug = $article->properties->category ?? '';
        $categoryColor = $category->properties->color ?? '#6B7280';
        $articleTags = $article->properties->tags ?? [];
        $hasGraphData = !empty($graphData['correction']) ||
                        !empty($graphData['timeline']['predecessors']) ||
                        !empty($graphData['timeline']['successors']) ||
                        !empty($graphData['contradictions']) ||
                        !empty($graphData['evidence']) ||
                        !empty($graphData['related']);
    @endphp

    <div class="grid gap-8 lg:grid-cols-3">
        {{-- Main Content --}}
        <div class="lg:col-span-2">
            {{-- Article Header --}}
            <article class="rounded-xl bg-white p-6 shadow-sm">
                {{-- Category Badge --}}
                @if($category && $categorySlug)
                    <a href="{{ route('articles.show', ['path' => $categorySlug]) }}"
                       class="inline-flex items-center rounded-full px-3 py-1 text-sm font-semibold uppercase tracking-wide"
                       style="background-color: {{ $categoryColor }}20; color: {{ $categoryColor }}">
                        {{ $category->properties->label ?? $category->name }}
                    </a>
                @endif

                {{-- Title --}}
                <h1 class="mt-4 text-3xl font-bold text-gray-900">{{ $article->properties->title }}</h1>

                {{-- Meta --}}
                <div class="mt-4 flex flex-wrap items-center gap-4 text-sm text-gray-500">
                    @if($article->properties->author ?? false)
                        <span class="flex items-center gap-1">
                            <x-lucide-user class="h-4 w-4" />
                            {{ $article->properties->author }}
                        </span>
                    @endif
                    <span class="flex items-center gap-1">
                        <x-lucide-calendar class="h-4 w-4" />
                        {{ \Carbon\Carbon::parse($article->properties->publishing_date ?? $article->created_at)->format('F j, Y') }}
                    </span>
                    <span class="flex items-center gap-1">
                        <x-lucide-eye class="h-4 w-4" />
                        {{ number_format($article->properties->views ?? 0) }} views
                    </span>
                </div>

                {{-- Tags --}}
                @if(count($articleTags) > 0)
                    <div class="mt-4 flex flex-wrap gap-2">
                        @foreach($articleTags as $tag)
                            @include('components.tag-chip', [
                                'tag' => $tag,
                                'tagData' => $tagMap[$tag->{'raisin:path'} ?? ''] ?? null
                            ])
                        @endforeach
                    </div>
                @endif

                {{-- Featured Image --}}
                @if($article->properties->imageUrl ?? false)
                    <img src="{{ $article->properties->imageUrl }}"
                         alt="{{ $article->properties->title }}"
                         class="mt-6 aspect-video w-full rounded-lg object-cover">
                @endif

                {{-- Body Content --}}
                <div class="prose mt-8 max-w-none">
                    {!! \GrahamCampbell\Markdown\Facades\Markdown::convert($article->properties->body ?? '') !!}
                </div>

                {{-- Actions --}}
                <div class="mt-8 flex items-center gap-3 border-t border-gray-200 pt-6">
                    <a href="{{ route('articles.edit', ['path' => $articlePath]) }}"
                       class="inline-flex items-center gap-2 rounded-lg border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50">
                        <x-lucide-edit class="h-4 w-4" />
                        Edit
                    </a>
                    <a href="{{ route('articles.move', ['path' => $articlePath]) }}"
                       class="inline-flex items-center gap-2 rounded-lg border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50">
                        <x-lucide-folder-input class="h-4 w-4" />
                        Move
                    </a>
                    <form action="{{ route('articles.destroy', ['path' => $articlePath]) }}"
                          method="POST"
                          onsubmit="return confirm('Are you sure you want to delete this article?')">
                        @csrf
                        @method('DELETE')
                        <button type="submit"
                                class="inline-flex items-center gap-2 rounded-lg border border-red-300 bg-white px-4 py-2 text-sm font-medium text-red-600 hover:bg-red-50">
                            <x-lucide-trash-2 class="h-4 w-4" />
                            Delete
                        </button>
                    </form>
                </div>
            </article>

            {{-- More in Category --}}
            @if(count($related) > 0)
                <section class="mt-8">
                    <h2 class="mb-4 text-xl font-bold text-gray-900">More in {{ $category->properties->label ?? $category->name }}</h2>
                    <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                        @foreach($related as $relatedArticle)
                            @include('components.article-card', [
                                'article' => $relatedArticle,
                                'categories' => $categories,
                                'tagMap' => $tagMap
                            ])
                        @endforeach
                    </div>
                </section>
            @endif
        </div>

        {{-- Sidebar with Graph Widgets --}}
        @if($hasGraphData)
            <div class="space-y-6">
                {{-- Correction Banner --}}
                @if(!empty($graphData['correction']))
                    @include('components.graph.correction-banner', ['correction' => $graphData['correction']])
                @endif

                {{-- Story Timeline --}}
                @if(!empty($graphData['timeline']['predecessors']) || !empty($graphData['timeline']['successors']))
                    @include('components.graph.story-timeline', ['timeline' => $graphData['timeline']])
                @endif

                {{-- Balanced View (Contradictions) --}}
                @if(!empty($graphData['contradictions']))
                    @include('components.graph.balanced-view', ['contradictions' => $graphData['contradictions']])
                @endif

                {{-- Evidence Sources --}}
                @if(!empty($graphData['evidence']))
                    @include('components.graph.evidence-sources', ['evidence' => $graphData['evidence']])
                @endif

                {{-- Smart Related --}}
                @if(!empty($graphData['related']))
                    @include('components.graph.smart-related', ['related' => $graphData['related']])
                @endif
            </div>
        @endif
    </div>
@endsection
