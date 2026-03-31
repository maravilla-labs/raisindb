@extends('layouts.app')

@section('title', 'Move: ' . ($article->properties->title ?? 'Article'))

@section('content')
    @php
        $articlePath = str_replace('/superbigshit/articles/', '', $article->path);
    @endphp

    <div class="mx-auto max-w-xl">
        <h1 class="mb-2 text-2xl font-bold text-gray-900">Move Article</h1>
        <p class="mb-8 text-gray-500">Select a new category for "{{ $article->properties->title }}"</p>

        <form action="{{ route('articles.move.update', ['path' => $articlePath]) }}" method="POST">
            @csrf
            @method('PUT')

            <div class="rounded-lg border border-gray-200 bg-white p-6">
                <label class="mb-4 block text-sm font-medium text-gray-700">Select Category</label>

                <div class="space-y-2">
                    @foreach($categories as $category)
                        @php
                            $slug = basename($category->path);
                            $isCurrentCategory = $slug === $currentCategory;
                            $color = $category->properties->color ?? '#6B7280';
                        @endphp
                        <label class="flex cursor-pointer items-center gap-3 rounded-lg border p-4 transition-colors
                                      {{ $isCurrentCategory ? 'border-blue-500 bg-blue-50' : 'border-gray-200 hover:bg-gray-50' }}">
                            <input type="radio"
                                   name="category"
                                   value="{{ $slug }}"
                                   {{ $isCurrentCategory ? 'checked' : '' }}
                                   class="h-4 w-4 text-blue-600 focus:ring-blue-500">
                            <span class="flex items-center gap-2">
                                <span class="inline-flex h-6 w-6 items-center justify-center rounded"
                                      style="background-color: {{ $color }}20">
                                    <x-lucide-folder class="h-3.5 w-3.5" style="color: {{ $color }}" />
                                </span>
                                <span class="font-medium text-gray-900">{{ $category->properties->label ?? $category->name }}</span>
                            </span>
                            @if($isCurrentCategory)
                                <span class="ml-auto text-xs text-blue-600">Current</span>
                            @endif
                        </label>
                    @endforeach
                </div>
            </div>

            <div class="mt-6 flex items-center justify-end gap-3">
                <a href="{{ route('articles.show', ['path' => $articlePath]) }}"
                   class="rounded-lg border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50">
                    Cancel
                </a>
                <button type="submit"
                        class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700">
                    Move Article
                </button>
            </div>
        </form>
    </div>
@endsection
