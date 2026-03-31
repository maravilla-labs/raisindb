@props(['connections' => [], 'availableArticles' => [], 'currentPath' => '', 'name' => 'connections'])

<div x-data="connectionPicker({
    initialConnections: {{ json_encode($connections) }},
    availableArticles: {{ json_encode($availableArticles) }},
    currentPath: '{{ $currentPath }}'
})" class="mt-1">
    {{-- Connection List --}}
    <div x-show="connections.length > 0" class="mb-3 space-y-2">
        <template x-for="(conn, index) in connections" :key="conn.targetPath">
            <div class="flex items-center gap-2 rounded-lg border border-gray-200 bg-gray-50 p-3">
                <span class="rounded-full px-2 py-0.5 text-xs font-medium"
                      :style="'background-color:' + (getRelationType(conn.relationType)?.color || '#6B7280') + '20; color:' + (getRelationType(conn.relationType)?.color || '#6B7280')">
                    <span x-text="getRelationType(conn.relationType)?.label || conn.relationType"></span>
                </span>
                <span class="flex-1 text-sm font-medium text-gray-900" x-text="conn.targetTitle"></span>
                <span class="text-xs text-gray-500" x-text="conn.weight + '%'"></span>
                <button type="button"
                        @click="openEditModal(index)"
                        class="text-gray-400 hover:text-gray-600">
                    <x-lucide-edit-2 class="h-4 w-4" />
                </button>
                <button type="button"
                        @click="removeConnection(index)"
                        class="text-gray-400 hover:text-red-600">
                    <x-lucide-x class="h-4 w-4" />
                </button>
            </div>
        </template>
    </div>

    {{-- Add Connection Button --}}
    <button type="button"
            @click="openAddModal()"
            class="inline-flex items-center gap-2 rounded-lg border border-dashed border-gray-300 px-3 py-2 text-sm text-gray-600 hover:border-gray-400 hover:text-gray-700">
        <x-lucide-plus class="h-4 w-4" />
        Add Connection
    </button>

    {{-- Connection Modal --}}
    <div x-show="modalOpen"
         x-transition
         class="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
         @click.self="modalOpen = false">
        <div class="w-full max-w-lg rounded-xl bg-white p-6 shadow-xl" @click.stop>
            <h3 class="text-lg font-semibold text-gray-900" x-text="editingIndex !== null ? 'Edit Connection' : 'Add Connection'"></h3>

            {{-- Article Search --}}
            <div class="mt-4">
                <label class="block text-sm font-medium text-gray-700">Target Article</label>
                <div class="relative mt-1">
                    <template x-if="!selectedArticle">
                        <div>
                            <input type="text"
                                   x-model="searchQuery"
                                   @focus="showDropdown = true"
                                   @blur="setTimeout(() => showDropdown = false, 150)"
                                   placeholder="Search for an article..."
                                   class="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
                            <div x-show="showDropdown && filteredArticles.length > 0"
                                 class="absolute z-10 mt-1 max-h-48 w-full overflow-auto rounded-lg border border-gray-200 bg-white shadow-lg">
                                <template x-for="article in filteredArticles" :key="article.path">
                                    <button type="button"
                                            @click="selectArticle(article)"
                                            class="w-full px-3 py-2 text-left text-sm hover:bg-gray-50">
                                        <span x-text="article.properties?.title || article.name"></span>
                                    </button>
                                </template>
                            </div>
                        </div>
                    </template>
                    <template x-if="selectedArticle">
                        <div class="flex items-center gap-2 rounded-lg bg-blue-50 px-3 py-2">
                            <span class="flex-1 text-sm font-medium text-blue-900" x-text="selectedArticle.properties?.title || selectedArticle.name"></span>
                            <button type="button" @click="selectedArticle = null" class="text-blue-600 hover:text-blue-800">
                                <x-lucide-x class="h-4 w-4" />
                            </button>
                        </div>
                    </template>
                </div>
            </div>

            {{-- Relation Type --}}
            <div class="mt-4">
                <label class="block text-sm font-medium text-gray-700">Relation Type</label>
                <select x-model="relationType"
                        class="mt-1 w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500">
                    <template x-for="type in relationTypes" :key="type.value">
                        <option :value="type.value" x-text="type.label + ' - ' + type.description"></option>
                    </template>
                </select>
            </div>

            {{-- Weight Slider --}}
            <div class="mt-4">
                <label class="block text-sm font-medium text-gray-700">
                    Weight: <span x-text="weight"></span>%
                </label>
                <input type="range"
                       x-model="weight"
                       min="0"
                       max="100"
                       step="5"
                       class="mt-1 w-full">
            </div>

            {{-- Editorial Note --}}
            <div class="mt-4">
                <label class="block text-sm font-medium text-gray-700">Editorial Note (optional)</label>
                <textarea x-model="editorialNote"
                          rows="2"
                          class="mt-1 w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"></textarea>
            </div>

            {{-- Modal Actions --}}
            <div class="mt-6 flex justify-end gap-3">
                <button type="button"
                        @click="modalOpen = false"
                        class="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50">
                    Cancel
                </button>
                <button type="button"
                        @click="saveConnection()"
                        :disabled="!selectedArticle"
                        class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:bg-gray-300">
                    Save
                </button>
            </div>
        </div>
    </div>

    {{-- Hidden input for form submission --}}
    <input type="hidden" name="{{ $name }}" x-ref="connectionsInput" :value="JSON.stringify(connections)">
</div>
