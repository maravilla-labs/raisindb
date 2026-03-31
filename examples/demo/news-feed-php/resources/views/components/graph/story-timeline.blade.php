@props(['timeline'])

<div class="rounded-lg border border-gray-200 bg-white p-4">
    <h3 class="flex items-center gap-2 font-semibold text-gray-900">
        <x-lucide-git-branch class="h-5 w-5 text-blue-600" />
        Story Timeline
    </h3>

    <div class="mt-4 space-y-3">
        {{-- Predecessors --}}
        @if(count($timeline['predecessors'] ?? []) > 0)
            <div>
                <p class="text-xs font-medium uppercase tracking-wide text-gray-500">Previously</p>
                <div class="mt-2 space-y-2">
                    @foreach($timeline['predecessors'] as $predecessor)
                        @php
                            $path = str_replace('/superbigshit/articles/', '', $predecessor->path);
                            $relationType = $predecessor->relation_type ?? 'continues';
                        @endphp
                        <a href="{{ route('articles.show', ['path' => $path]) }}"
                           class="block rounded-lg border border-gray-100 p-3 transition-colors hover:bg-gray-50">
                            <span class="text-xs text-gray-500">{{ ucfirst($relationType) }}</span>
                            <p class="mt-1 text-sm font-medium text-gray-900">
                                {{ $predecessor->properties->title ?? $predecessor->name }}
                            </p>
                        </a>
                    @endforeach
                </div>
            </div>
        @endif

        {{-- Current Article Indicator --}}
        <div class="flex items-center gap-2 rounded-lg bg-blue-50 p-3">
            <div class="h-2 w-2 rounded-full bg-blue-600"></div>
            <span class="text-sm font-medium text-blue-900">Current Article</span>
        </div>

        {{-- Successors --}}
        @if(count($timeline['successors'] ?? []) > 0)
            <div>
                <p class="text-xs font-medium uppercase tracking-wide text-gray-500">Later</p>
                <div class="mt-2 space-y-2">
                    @foreach($timeline['successors'] as $successor)
                        @php
                            $path = str_replace('/superbigshit/articles/', '', $successor->path);
                            $relationType = $successor->relation_type ?? 'continues';
                        @endphp
                        <a href="{{ route('articles.show', ['path' => $path]) }}"
                           class="block rounded-lg border border-gray-100 p-3 transition-colors hover:bg-gray-50">
                            <span class="text-xs text-gray-500">{{ ucfirst($relationType) }}</span>
                            <p class="mt-1 text-sm font-medium text-gray-900">
                                {{ $successor->properties->title ?? $successor->name }}
                            </p>
                        </a>
                    @endforeach
                </div>
            </div>
        @endif
    </div>
</div>
