@props(['article', 'categories' => [], 'tagMap' => [], 'featured' => false])

@php
    $categorySlug = $article->properties->category ?? '';
    $category = collect($categories)->first(fn($c) => basename($c->path) === $categorySlug);
    $articlePath = str_replace('/superbigshit/articles/', '', $article->path);
    $tags = $article->properties->tags ?? [];
@endphp

<article {{ $attributes->merge(['class' => 'group overflow-hidden rounded-xl border border-gray-200 bg-white shadow-sm transition-shadow hover:shadow-md']) }}>
    @if($article->properties->imageUrl ?? false)
        <a href="{{ route('articles.show', ['path' => $articlePath]) }}" class="block overflow-hidden">
            <img src="{{ $article->properties->imageUrl }}"
                 alt="{{ $article->properties->title }}"
                 class="aspect-video w-full object-cover transition-transform duration-300 group-hover:scale-105">
        </a>
    @endif

    <div class="p-5">
        @if($category)
            <a href="{{ route('articles.show', ['path' => $categorySlug]) }}"
               class="inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-semibold uppercase tracking-wide"
               style="background-color: {{ $category->properties->color ?? '#6B7280' }}20; color: {{ $category->properties->color ?? '#6B7280' }}">
                {{ $category->properties->label ?? $category->name }}
            </a>
        @endif

        <h3 class="mt-3 text-lg font-semibold text-gray-900">
            <a href="{{ route('articles.show', ['path' => $articlePath]) }}" class="hover:text-blue-600">
                {{ $article->properties->title }}
            </a>
        </h3>

        @if($article->properties->excerpt ?? false)
            <p class="mt-2 line-clamp-2 text-sm text-gray-600">
                {{ $article->properties->excerpt }}
            </p>
        @endif

        @if(count($tags) > 0)
            <div class="mt-3 flex flex-wrap gap-1">
                @foreach(array_slice($tags, 0, 3) as $tag)
                    @include('components.tag-chip', [
                        'tag' => $tag,
                        'tagData' => $tagMap[$tag->{'raisin:path'} ?? ''] ?? null,
                        'size' => 'sm'
                    ])
                @endforeach
            </div>
        @endif

        <div class="mt-4 flex items-center gap-3 text-xs text-gray-500">
            @if($article->properties->author ?? false)
                <span>{{ $article->properties->author }}</span>
            @endif
            <span class="flex items-center gap-1">
                <x-lucide-clock class="h-3 w-3" />
                {{ \Carbon\Carbon::parse($article->properties->publishing_date ?? $article->created_at)->format('M j, Y') }}
            </span>
            <span class="flex items-center gap-1">
                <x-lucide-eye class="h-3 w-3" />
                {{ number_format($article->properties->views ?? 0) }}
            </span>
        </div>
    </div>
</article>
