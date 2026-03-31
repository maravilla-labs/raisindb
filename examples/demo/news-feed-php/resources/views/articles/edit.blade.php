@extends('layouts.app')

@section('title', 'Edit: ' . ($article->properties->title ?? 'Article'))

@section('content')
    @php
        $articlePath = str_replace('/superbigshit/articles/', '', $article->path);
    @endphp

    <div class="mx-auto max-w-4xl">
        <h1 class="mb-8 text-2xl font-bold text-gray-900">Edit Article</h1>

        <form action="{{ route('articles.update', ['path' => $articlePath]) }}" method="POST" class="space-y-6">
            @csrf
            @method('PUT')

            @include('components.forms.article-form', [
                'article' => $article,
                'categories' => $categories,
                'tags' => $tags,
                'availableArticles' => $availableArticles ?? [],
                'incomingConnections' => $incomingConnections ?? []
            ])

            <div class="flex items-center justify-end gap-3 border-t border-gray-200 pt-6">
                <a href="{{ route('articles.show', ['path' => $articlePath]) }}"
                   class="rounded-lg border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50">
                    Cancel
                </a>
                <button type="submit"
                        class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700">
                    Save Changes
                </button>
            </div>
        </form>
    </div>
@endsection
