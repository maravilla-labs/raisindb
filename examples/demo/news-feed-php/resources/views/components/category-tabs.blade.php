@props(['categories' => []])

<nav class="border-b border-gray-200 bg-white">
    <div class="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
        <div class="flex gap-1 overflow-x-auto py-2">
            {{-- All Articles Tab --}}
            <a href="{{ route('home') }}"
               class="whitespace-nowrap rounded-lg px-4 py-2 text-sm font-medium transition-colors
                      {{ request()->routeIs('home') ? 'bg-gray-900 text-white' : 'text-gray-600 hover:bg-gray-100 hover:text-gray-900' }}">
                All
            </a>

            {{-- Category Tabs --}}
            @foreach($categories as $category)
                @php
                    $slug = basename($category->path);
                    $isActive = request()->is("articles/{$slug}") || request()->is("articles/{$slug}/*");
                    $color = $category->properties->color ?? '#6B7280';
                @endphp
                <a href="{{ route('articles.show', ['path' => $slug]) }}"
                   class="whitespace-nowrap rounded-lg px-4 py-2 text-sm font-medium transition-colors
                          {{ $isActive ? 'text-white' : 'text-gray-600 hover:bg-gray-100 hover:text-gray-900' }}"
                   @if($isActive) style="background-color: {{ $color }}" @endif>
                    {{ $category->properties->label ?? $category->name }}
                </a>
            @endforeach
        </div>
    </div>
</nav>
