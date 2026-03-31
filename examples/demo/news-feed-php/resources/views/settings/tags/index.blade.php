@extends('layouts.app')

@section('title', 'Manage Tags')

@section('content')
    <div class="mx-auto max-w-4xl">
        {{-- Header --}}
        <div class="mb-8 flex items-center justify-between">
            <div>
                <h1 class="text-2xl font-bold text-gray-900">Tags</h1>
                <p class="mt-1 text-gray-500">Manage hierarchical tags for articles</p>
            </div>
            <div class="flex items-center gap-3">
                <a href="{{ route('settings.categories.index') }}"
                   class="rounded-lg border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50">
                    Manage Categories
                </a>
            </div>
        </div>

        {{-- Tag Tree --}}
        <div class="rounded-lg border border-gray-200 bg-white">
            @if(count($tagTree) > 0)
                <div class="divide-y divide-gray-100">
                    @foreach($tagTree as $tag)
                        @include('settings.tags._tag-row', ['tag' => $tag, 'depth' => 0])
                    @endforeach
                </div>
            @else
                <div class="p-8 text-center text-gray-500">
                    <x-lucide-tags class="mx-auto h-12 w-12 text-gray-300" />
                    <p class="mt-4">No tags yet. Create your first tag below.</p>
                </div>
            @endif
        </div>

        {{-- Create New Tag --}}
        <div class="mt-6 rounded-lg border-2 border-dashed border-gray-300 p-6" x-data="{ showForm: false }">
            <button x-show="!showForm"
                    @click="showForm = true"
                    class="flex w-full items-center justify-center gap-2 text-gray-500 hover:text-gray-700">
                <x-lucide-plus class="h-5 w-5" />
                <span>Add Root Tag</span>
            </button>

            <form x-show="showForm"
                  action="{{ route('settings.tags.store') }}"
                  method="POST"
                  class="space-y-4">
                @csrf
                <div class="grid gap-4 sm:grid-cols-3">
                    <div>
                        <label class="block text-sm font-medium text-gray-700">Label</label>
                        <input type="text"
                               name="label"
                               required
                               placeholder="Tag name"
                               class="mt-1 w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
                    </div>
                    <div>
                        <label class="block text-sm font-medium text-gray-700">Icon (Lucide)</label>
                        <input type="text"
                               name="icon"
                               placeholder="e.g., tag, folder, star"
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
                <div class="flex justify-end gap-2">
                    <button type="button"
                            @click="showForm = false"
                            class="rounded-lg border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50">
                        Cancel
                    </button>
                    <button type="submit"
                            class="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700">
                        Create Tag
                    </button>
                </div>
            </form>
        </div>
    </div>
@endsection
