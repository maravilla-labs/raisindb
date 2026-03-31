@props(['selectedTags' => [], 'availableTags' => [], 'name' => 'tags'])

<div x-data="tagPicker({
    initialTags: {{ json_encode($selectedTags) }},
    availableTags: {{ json_encode($availableTags) }}
})" class="relative mt-1">
    {{-- Selected Tags --}}
    <div x-show="selectedTags.length > 0" class="mb-2 flex flex-wrap gap-1.5">
        <template x-for="(tag, index) in selectedTags" :key="tag['raisin:path']">
            <span class="inline-flex items-center gap-1 rounded-full bg-gray-100 px-2.5 py-1 text-sm">
                <span x-text="getTagData(tag['raisin:path'])?.properties?.label || getTagData(tag['raisin:path'])?.name || tag['raisin:path'].split('/').pop()"></span>
                <button type="button" @click="removeTag(index)" class="text-gray-400 hover:text-gray-600">
                    <x-lucide-x class="h-3 w-3" />
                </button>
            </span>
        </template>
    </div>

    {{-- Search Input --}}
    <div class="relative">
        <x-lucide-search class="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-400" />
        <input type="text"
               x-model="searchQuery"
               @focus="handleFocus()"
               @blur="handleBlur()"
               placeholder="Search tags..."
               class="w-full rounded-lg border border-gray-300 py-2 pl-9 pr-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
    </div>

    {{-- Dropdown --}}
    <div x-show="isOpen"
         x-transition
         class="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-lg border border-gray-200 bg-white shadow-lg">
        <template x-if="filteredTags.length > 0">
            <div>
                <template x-for="tag in filteredTags" :key="tag.path">
                    <button type="button"
                            @click="addTag(tag)"
                            class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-gray-50">
                        <span class="h-4 w-4 rounded-full" :style="'background-color:' + (tag.properties?.color || '#6B7280')"></span>
                        <span class="flex-1" x-text="tag.properties?.label || tag.name"></span>
                        <span class="text-xs text-gray-400" x-text="tag.path.split('/').slice(-2, -1)[0] || ''"></span>
                    </button>
                </template>
            </div>
        </template>
        <template x-if="filteredTags.length === 0 && searchQuery.trim()">
            <div class="px-3 py-4 text-center text-sm text-gray-500">
                No tags found matching "<span x-text="searchQuery"></span>"
            </div>
        </template>
        <template x-if="filteredTags.length === 0 && !searchQuery.trim()">
            <div class="px-3 py-4 text-center text-sm text-gray-500">
                Type to search for tags
            </div>
        </template>
    </div>

    {{-- Hidden input for form submission --}}
    <input type="hidden" name="{{ $name }}" x-ref="tagsInput" :value="JSON.stringify(selectedTags)">
</div>
