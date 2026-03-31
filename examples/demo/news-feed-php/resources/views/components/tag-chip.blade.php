@props(['tag', 'tagData' => null, 'size' => 'md', 'removable' => false, 'onRemove' => null])

@php
    $path = $tag->{'raisin:path'} ?? '';
    $label = $tagData->properties->label ?? $tagData->name ?? basename($path);
    $color = $tagData->properties->color ?? '#6B7280';
    $icon = $tagData->properties->icon ?? null;

    $sizeClasses = match($size) {
        'sm' => 'text-xs px-2 py-0.5',
        'lg' => 'text-sm px-3 py-1.5',
        default => 'text-xs px-2.5 py-1',
    };

    // Extract relative path for URL
    $tagPath = str_replace('/superbigshit/tags/', '', $path);
@endphp

@if($removable)
    <span class="inline-flex items-center gap-1 rounded-full {{ $sizeClasses }}"
          style="background-color: {{ $color }}15; color: {{ $color }}">
        @if($icon)
            <x-dynamic-component :component="'lucide-' . $icon" class="h-3 w-3" />
        @endif
        <span>{{ $label }}</span>
        <button type="button"
                @if($onRemove) @click="{{ $onRemove }}" @endif
                class="ml-0.5 rounded-full p-0.5 hover:bg-gray-200">
            <x-lucide-x class="h-3 w-3" />
        </button>
    </span>
@else
    <a href="{{ route('search', ['tag' => $tagPath]) }}"
       class="inline-flex items-center gap-1 rounded-full {{ $sizeClasses }} transition-opacity hover:opacity-80"
       style="background-color: {{ $color }}15; color: {{ $color }}">
        @if($icon)
            <x-dynamic-component :component="'lucide-' . $icon" class="h-3 w-3" />
        @endif
        <span>{{ $label }}</span>
    </a>
@endif
