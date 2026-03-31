@extends('layouts.app')

@section('title', 'Manage Categories')

@section('content')
    <div class="mx-auto max-w-4xl">
        {{-- Header --}}
        <div class="mb-8 flex items-center justify-between">
            <div>
                <h1 class="text-2xl font-bold text-gray-900">Categories</h1>
                <p class="mt-1 text-gray-500">Manage article categories</p>
            </div>
            <div class="flex items-center gap-3">
                <a href="{{ route('settings.tags.index') }}"
                   class="rounded-lg border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50">
                    Manage Tags
                </a>
            </div>
        </div>

        {{-- Category List --}}
        <div class="space-y-4">
            @foreach($settingsCategories as $category)
                @php
                    $slug = basename($category->path);
                    $color = $category->properties->color ?? '#6B7280';
                @endphp
                <div class="rounded-lg border border-gray-200 bg-white p-4" x-data="{ editing: false }">
                    <div x-show="!editing" class="flex items-center gap-4">
                        <span class="inline-flex h-10 w-10 items-center justify-center rounded-lg"
                              style="background-color: {{ $color }}20">
                            <x-lucide-folder class="h-5 w-5" style="color: {{ $color }}" />
                        </span>
                        <div class="flex-1">
                            <h3 class="font-semibold text-gray-900">{{ $category->properties->label ?? $category->name }}</h3>
                            <p class="text-sm text-gray-500">{{ $category->article_count ?? 0 }} articles</p>
                        </div>
                        <button type="button"
                                @click="editing = true"
                                class="rounded-lg p-2 text-gray-400 hover:bg-gray-100 hover:text-gray-600">
                            <x-lucide-edit-2 class="h-4 w-4" />
                        </button>
                        @if(($category->article_count ?? 0) === 0)
                            <form action="{{ route('settings.categories.destroy', ['slug' => $slug]) }}"
                                  method="POST"
                                  onsubmit="return confirm('Delete this category?')">
                                @csrf
                                @method('DELETE')
                                <button type="submit"
                                        class="rounded-lg p-2 text-gray-400 hover:bg-red-100 hover:text-red-600">
                                    <x-lucide-trash-2 class="h-4 w-4" />
                                </button>
                            </form>
                        @endif
                    </div>

                    {{-- Edit Form --}}
                    <form x-show="editing"
                          action="{{ route('settings.categories.update', ['slug' => $slug]) }}"
                          method="POST"
                          class="space-y-4">
                        @csrf
                        @method('PUT')
                        <div class="grid gap-4 sm:grid-cols-2">
                            <div>
                                <label class="block text-sm font-medium text-gray-700">Label</label>
                                <input type="text"
                                       name="label"
                                       value="{{ $category->properties->label ?? $category->name }}"
                                       required
                                       class="mt-1 w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
                            </div>
                            <div>
                                <label class="block text-sm font-medium text-gray-700">Color</label>
                                <input type="color"
                                       name="color"
                                       value="{{ $color }}"
                                       class="mt-1 h-10 w-full rounded-lg border border-gray-300">
                            </div>
                        </div>
                        <div>
                            <label class="block text-sm font-medium text-gray-700">Description</label>
                            <textarea name="description"
                                      rows="2"
                                      class="mt-1 w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">{{ $category->properties->description ?? '' }}</textarea>
                        </div>
                        <div class="flex justify-end gap-2">
                            <button type="button"
                                    @click="editing = false"
                                    class="rounded-lg border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50">
                                Cancel
                            </button>
                            <button type="submit"
                                    class="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700">
                                Save
                            </button>
                        </div>
                    </form>
                </div>
            @endforeach
        </div>

        {{-- Create New Category --}}
        <div class="mt-6 rounded-lg border-2 border-dashed border-gray-300 p-6" x-data="{ showForm: false }">
            <button x-show="!showForm"
                    @click="showForm = true"
                    class="flex w-full items-center justify-center gap-2 text-gray-500 hover:text-gray-700">
                <x-lucide-plus class="h-5 w-5" />
                <span>Add New Category</span>
            </button>

            <form x-show="showForm"
                  action="{{ route('settings.categories.store') }}"
                  method="POST"
                  class="space-y-4">
                @csrf
                <div class="grid gap-4 sm:grid-cols-2">
                    <div>
                        <label class="block text-sm font-medium text-gray-700">Label</label>
                        <input type="text"
                               name="label"
                               required
                               placeholder="Category name"
                               class="mt-1 w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
                    </div>
                    <div>
                        <label class="block text-sm font-medium text-gray-700">Color</label>
                        <input type="color"
                               name="color"
                               value="#6B7280"
                               class="mt-1 h-10 w-full rounded-lg border border-gray-300">
                    </div>
                </div>
                <div>
                    <label class="block text-sm font-medium text-gray-700">Description</label>
                    <textarea name="description"
                              rows="2"
                              placeholder="Optional description"
                              class="mt-1 w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"></textarea>
                </div>
                <div class="flex justify-end gap-2">
                    <button type="button"
                            @click="showForm = false"
                            class="rounded-lg border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50">
                        Cancel
                    </button>
                    <button type="submit"
                            class="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700">
                        Create Category
                    </button>
                </div>
            </form>
        </div>
    </div>
@endsection
