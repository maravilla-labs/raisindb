@props(['article' => null, 'categories' => [], 'tags' => [], 'availableArticles' => [], 'incomingConnections' => []])

@php
    $articleTags = $article ? ($article->properties->tags ?? []) : [];
    $articleConnections = $article ? ($article->properties->connections ?? []) : [];
    $currentCategory = $article ? ($article->properties->category ?? '') : '';
    $keywords = $article ? implode(', ', $article->properties->keywords ?? []) : '';
@endphp

<div class="space-y-6 rounded-lg border border-gray-200 bg-white p-6">
    {{-- Basic Info --}}
    <div class="grid gap-6 md:grid-cols-2">
        {{-- Title --}}
        <div x-data="slugGenerator()" class="md:col-span-2">
            <label for="title" class="block text-sm font-medium text-gray-700">Title *</label>
            <input type="text"
                   name="title"
                   id="title"
                   required
                   value="{{ old('title', $article->properties->title ?? '') }}"
                   @input="generateSlug($event.target.value)"
                   class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
            @error('title')
                <p class="mt-1 text-sm text-red-600">{{ $message }}</p>
            @enderror
        </div>

        {{-- Slug --}}
        <div x-data="slugGenerator()">
            <label for="slug" class="block text-sm font-medium text-gray-700">Slug *</label>
            <input type="text"
                   name="slug"
                   id="slug"
                   required
                   x-ref="slugInput"
                   @input="markSlugEdited()"
                   value="{{ old('slug', $article->properties->slug ?? '') }}"
                   class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
            @error('slug')
                <p class="mt-1 text-sm text-red-600">{{ $message }}</p>
            @enderror
        </div>

        {{-- Category --}}
        <div>
            <label for="category" class="block text-sm font-medium text-gray-700">Category *</label>
            <select name="category"
                    id="category"
                    required
                    class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
                <option value="">Select a category...</option>
                @foreach($categories as $category)
                    @php $slug = basename($category->path); @endphp
                    <option value="{{ $slug }}" {{ old('category', $currentCategory) === $slug ? 'selected' : '' }}>
                        {{ $category->properties->label ?? $category->name }}
                    </option>
                @endforeach
            </select>
            @error('category')
                <p class="mt-1 text-sm text-red-600">{{ $message }}</p>
            @enderror
        </div>
    </div>

    {{-- Excerpt --}}
    <div>
        <label for="excerpt" class="block text-sm font-medium text-gray-700">Excerpt</label>
        <textarea name="excerpt"
                  id="excerpt"
                  rows="2"
                  class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">{{ old('excerpt', $article->properties->excerpt ?? '') }}</textarea>
        <p class="mt-1 text-xs text-gray-500">A brief summary for previews</p>
    </div>

    {{-- Body (Markdown) --}}
    <div x-data="markdownPreview()">
        <div class="flex items-center justify-between">
            <label for="body" class="block text-sm font-medium text-gray-700">Body (Markdown)</label>
            <button type="button"
                    @click="togglePreview()"
                    class="text-sm text-blue-600 hover:text-blue-800">
                <span x-show="!showPreview">Preview</span>
                <span x-show="showPreview">Edit</span>
            </button>
        </div>
        <div x-show="!showPreview">
            <textarea name="body"
                      id="body"
                      rows="12"
                      x-ref="bodyInput"
                      class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">{{ old('body', $article->properties->body ?? '') }}</textarea>
        </div>
        <div x-show="showPreview" class="prose mt-1 min-h-[200px] rounded-lg border border-gray-300 bg-gray-50 p-4">
            <div x-html="$refs.bodyInput ? window.marked?.parse($refs.bodyInput.value) || $refs.bodyInput.value : ''"></div>
        </div>
    </div>

    {{-- Tags --}}
    <div>
        <label class="block text-sm font-medium text-gray-700">Tags</label>
        @include('components.forms.tag-picker', [
            'selectedTags' => $articleTags,
            'availableTags' => $tags
        ])
    </div>

    {{-- Connections --}}
    @if(count($availableArticles) > 0)
        <div>
            <label class="block text-sm font-medium text-gray-700">Connections</label>
            @include('components.forms.connection-picker', [
                'connections' => $articleConnections,
                'availableArticles' => $availableArticles,
                'currentPath' => $article ? $article->path : ''
            ])
        </div>
    @endif

    {{-- Incoming Connections --}}
    @if(count($incomingConnections) > 0)
        <div>
            <label class="block text-sm font-medium text-gray-700">Incoming Connections</label>
            <div class="mt-2 space-y-2">
                @foreach($incomingConnections as $conn)
                    @php $path = str_replace('/superbigshit/articles/', '', $conn->path); @endphp
                    <div class="flex items-center gap-2 rounded-lg bg-gray-50 px-3 py-2 text-sm">
                        <x-lucide-arrow-left class="h-4 w-4 text-gray-400" />
                        <span class="text-gray-500">{{ $conn->relation_type ?? 'references' }}</span>
                        <a href="{{ route('articles.show', ['path' => $path]) }}" class="font-medium text-blue-600 hover:text-blue-800">
                            {{ $conn->properties->title ?? $conn->name }}
                        </a>
                    </div>
                @endforeach
            </div>
        </div>
    @endif

    {{-- Keywords --}}
    <div>
        <label for="keywords" class="block text-sm font-medium text-gray-700">Keywords</label>
        <input type="text"
               name="keywords"
               id="keywords"
               value="{{ old('keywords', $keywords) }}"
               placeholder="keyword1, keyword2, keyword3"
               class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
        <p class="mt-1 text-xs text-gray-500">Comma-separated keywords for search</p>
    </div>

    {{-- Author & Date --}}
    <div class="grid gap-6 md:grid-cols-2">
        <div>
            <label for="author" class="block text-sm font-medium text-gray-700">Author</label>
            <input type="text"
                   name="author"
                   id="author"
                   value="{{ old('author', $article->properties->author ?? '') }}"
                   class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
        </div>
        <div>
            <label for="publishing_date" class="block text-sm font-medium text-gray-700">Publishing Date</label>
            <input type="datetime-local"
                   name="publishing_date"
                   id="publishing_date"
                   value="{{ old('publishing_date', $article ? \Carbon\Carbon::parse($article->properties->publishing_date ?? now())->format('Y-m-d\TH:i') : now()->format('Y-m-d\TH:i')) }}"
                   class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
        </div>
    </div>

    {{-- Image URL --}}
    <div x-data="{ imageUrl: '{{ old('imageUrl', $article->properties->imageUrl ?? '') }}' }">
        <label for="imageUrl" class="block text-sm font-medium text-gray-700">Image URL</label>
        <input type="url"
               name="imageUrl"
               id="imageUrl"
               x-model="imageUrl"
               value="{{ old('imageUrl', $article->properties->imageUrl ?? '') }}"
               placeholder="https://example.com/image.jpg"
               class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
        <template x-if="imageUrl">
            <img :src="imageUrl" alt="Preview" class="mt-2 h-32 rounded-lg object-cover">
        </template>
    </div>

    {{-- Featured & Status --}}
    <div class="grid gap-6 md:grid-cols-2">
        <div class="flex items-center gap-2">
            <input type="checkbox"
                   name="featured"
                   id="featured"
                   value="1"
                   {{ old('featured', $article->properties->featured ?? false) ? 'checked' : '' }}
                   class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500">
            <label for="featured" class="text-sm font-medium text-gray-700">Featured Article</label>
        </div>

        <div>
            <label class="block text-sm font-medium text-gray-700">Status</label>
            <div class="mt-2 flex gap-4">
                <label class="flex items-center gap-2">
                    <input type="radio"
                           name="status"
                           value="draft"
                           {{ old('status', $article->properties->status ?? 'draft') === 'draft' ? 'checked' : '' }}
                           class="h-4 w-4 border-gray-300 text-blue-600 focus:ring-blue-500">
                    <span class="text-sm text-gray-700">Draft</span>
                </label>
                <label class="flex items-center gap-2">
                    <input type="radio"
                           name="status"
                           value="published"
                           {{ old('status', $article->properties->status ?? 'draft') === 'published' ? 'checked' : '' }}
                           class="h-4 w-4 border-gray-300 text-blue-600 focus:ring-blue-500">
                    <span class="text-sm text-gray-700">Published</span>
                </label>
            </div>
        </div>
    </div>
</div>
