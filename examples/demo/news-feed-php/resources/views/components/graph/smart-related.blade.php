@props(['related'])

@php
    $relationLabels = [
        'similar-to' => 'Similar',
        'see-also' => 'See Also',
        'updates' => 'Update',
    ];
    $relationColors = [
        'similar-to' => '#6B7280',
        'see-also' => '#6B7280',
        'updates' => '#8B5CF6',
    ];
@endphp

<div class="rounded-lg border border-gray-200 bg-white p-4">
    <h3 class="flex items-center gap-2 font-semibold text-gray-900">
        <x-lucide-sparkles class="h-5 w-5 text-amber-500" />
        Smart Related
    </h3>
    <p class="mt-1 text-xs text-gray-500">Recommended based on content connections</p>

    <div class="mt-4 space-y-2">
        @foreach($related as $article)
            @php
                $path = str_replace('/superbigshit/articles/', '', $article->path);
                $relationType = $article->relation_type ?? 'similar-to';
                $relationLabel = $relationLabels[$relationType] ?? ucfirst(str_replace('-', ' ', $relationType));
                $weight = ($article->weight ?? 0.5) * 100;
            @endphp
            <a href="{{ route('articles.show', ['path' => $path]) }}"
               class="block rounded-lg border border-gray-100 p-3 transition-colors hover:bg-gray-50">
                <div class="flex items-center gap-2">
                    <span class="rounded-full px-2 py-0.5 text-xs font-medium"
                          style="background-color: {{ $relationColors[$relationType] ?? '#6B7280' }}15; color: {{ $relationColors[$relationType] ?? '#6B7280' }}">
                        {{ $relationLabel }}
                    </span>
                    <span class="text-xs text-gray-400">{{ round($weight) }}%</span>
                </div>
                <p class="mt-2 text-sm font-medium text-gray-900">
                    {{ $article->properties->title ?? $article->name }}
                </p>
            </a>
        @endforeach
    </div>
</div>
