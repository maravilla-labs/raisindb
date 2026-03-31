@props(['correction'])

@php
    $correctionPath = str_replace('/superbigshit/articles/', '', $correction->path);
@endphp

<div class="rounded-lg border border-amber-200 bg-amber-50 p-4">
    <div class="flex items-start gap-3">
        <x-lucide-alert-triangle class="h-5 w-5 flex-shrink-0 text-amber-600" />
        <div>
            <h3 class="font-medium text-amber-800">This article has been corrected</h3>
            <p class="mt-1 text-sm text-amber-700">
                A newer version with corrections is available:
            </p>
            <a href="{{ route('articles.show', ['path' => $correctionPath]) }}"
               class="mt-2 inline-flex items-center gap-1 text-sm font-medium text-amber-700 hover:text-amber-900">
                {{ $correction->properties->title ?? $correction->name }}
                <x-lucide-arrow-right class="h-4 w-4" />
            </a>
        </div>
    </div>
</div>
