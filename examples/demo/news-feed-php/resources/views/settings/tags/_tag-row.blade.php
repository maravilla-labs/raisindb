@php
    $color = $tag->properties->color ?? '#6B7280';
    $icon = $tag->properties->icon ?? 'tag';
    $label = $tag->properties->label ?? $tag->name;
    $hasChildren = !empty($tag->children);
    $tagPath = str_replace('/superbigshit/tags/', '', $tag->path);
@endphp

<div x-data="{ expanded: false, editing: false, addingChild: false }">
    <div class="flex items-center gap-3 px-4 py-3 hover:bg-gray-50"
         style="padding-left: {{ ($depth * 24) + 16 }}px">
        {{-- Expand/Collapse Button --}}
        @if($hasChildren)
            <button type="button"
                    @click="expanded = !expanded"
                    class="text-gray-400 hover:text-gray-600">
                <x-lucide-chevron-right class="h-4 w-4 transition-transform" x-bind:class="expanded ? 'rotate-90' : ''" />
            </button>
        @else
            <span class="w-4"></span>
        @endif

        {{-- Tag Icon & Label --}}
        <span class="flex items-center gap-2">
            <span class="inline-flex h-6 w-6 items-center justify-center rounded"
                  style="background-color: {{ $color }}20">
                <x-dynamic-component :component="'lucide-' . $icon" class="h-3.5 w-3.5" style="color: {{ $color }}" />
            </span>
            <span class="font-medium text-gray-900">{{ $label }}</span>
        </span>

        {{-- Actions --}}
        <div class="ml-auto flex items-center gap-1">
            <button type="button"
                    @click="addingChild = !addingChild"
                    class="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
                    title="Add child tag">
                <x-lucide-plus class="h-4 w-4" />
            </button>
            <button type="button"
                    @click="editing = !editing"
                    class="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
                    title="Edit tag">
                <x-lucide-edit-2 class="h-4 w-4" />
            </button>
            <form action="{{ route('settings.tags.destroy', ['path' => $tagPath]) }}"
                  method="POST"
                  onsubmit="return confirm('Delete this tag and all children?')"
                  class="inline">
                @csrf
                @method('DELETE')
                <button type="submit"
                        class="rounded p-1 text-gray-400 hover:bg-red-100 hover:text-red-600"
                        title="Delete tag">
                    <x-lucide-trash-2 class="h-4 w-4" />
                </button>
            </form>
        </div>
    </div>

    {{-- Edit Form --}}
    <div x-show="editing" class="border-t border-gray-100 bg-gray-50 px-4 py-3" style="padding-left: {{ ($depth * 24) + 40 }}px">
        <form action="{{ route('settings.tags.update', ['path' => $tagPath]) }}"
              method="POST"
              class="flex items-end gap-3">
            @csrf
            @method('PUT')
            <div class="flex-1">
                <label class="block text-xs font-medium text-gray-500">Label</label>
                <input type="text"
                       name="label"
                       value="{{ $label }}"
                       required
                       class="mt-1 w-full rounded border border-gray-300 px-2 py-1 text-sm focus:border-blue-500 focus:outline-none">
            </div>
            <div class="w-24">
                <label class="block text-xs font-medium text-gray-500">Icon</label>
                <input type="text"
                       name="icon"
                       value="{{ $icon }}"
                       class="mt-1 w-full rounded border border-gray-300 px-2 py-1 text-sm focus:border-blue-500 focus:outline-none">
            </div>
            <div class="w-16">
                <label class="block text-xs font-medium text-gray-500">Color</label>
                <input type="color"
                       name="color"
                       value="{{ $color }}"
                       class="mt-1 h-7 w-full rounded border border-gray-300">
            </div>
            <button type="submit"
                    class="rounded bg-blue-600 px-3 py-1 text-sm font-medium text-white hover:bg-blue-700">
                Save
            </button>
            <button type="button"
                    @click="editing = false"
                    class="rounded border border-gray-300 px-3 py-1 text-sm font-medium text-gray-700 hover:bg-gray-100">
                Cancel
            </button>
        </form>
    </div>

    {{-- Add Child Form --}}
    <div x-show="addingChild" class="border-t border-gray-100 bg-blue-50 px-4 py-3" style="padding-left: {{ ($depth * 24) + 40 }}px">
        <form action="{{ route('settings.tags.store') }}"
              method="POST"
              class="flex items-end gap-3">
            @csrf
            <input type="hidden" name="parent_path" value="{{ $tag->path }}">
            <div class="flex-1">
                <label class="block text-xs font-medium text-gray-500">New Child Tag</label>
                <input type="text"
                       name="label"
                       required
                       placeholder="Tag name"
                       class="mt-1 w-full rounded border border-gray-300 px-2 py-1 text-sm focus:border-blue-500 focus:outline-none">
            </div>
            <div class="w-24">
                <label class="block text-xs font-medium text-gray-500">Icon</label>
                <input type="text"
                       name="icon"
                       placeholder="tag"
                       class="mt-1 w-full rounded border border-gray-300 px-2 py-1 text-sm focus:border-blue-500 focus:outline-none">
            </div>
            <div class="w-16">
                <label class="block text-xs font-medium text-gray-500">Color</label>
                <input type="color"
                       name="color"
                       value="#6B7280"
                       class="mt-1 h-7 w-full rounded border border-gray-300">
            </div>
            <button type="submit"
                    class="rounded bg-blue-600 px-3 py-1 text-sm font-medium text-white hover:bg-blue-700">
                Add
            </button>
            <button type="button"
                    @click="addingChild = false"
                    class="rounded border border-gray-300 px-3 py-1 text-sm font-medium text-gray-700 hover:bg-gray-100">
                Cancel
            </button>
        </form>
    </div>

    {{-- Children --}}
    @if($hasChildren)
        <div x-show="expanded" class="border-t border-gray-100">
            @foreach($tag->children as $child)
                @include('settings.tags._tag-row', ['tag' => $child, 'depth' => $depth + 1])
            @endforeach
        </div>
    @endif
</div>
