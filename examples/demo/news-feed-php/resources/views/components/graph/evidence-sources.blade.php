@props(['evidence'])

<div class="rounded-lg border border-gray-200 bg-white p-4">
    <h3 class="flex items-center gap-2 font-semibold text-gray-900">
        <x-lucide-file-check class="h-5 w-5 text-green-600" />
        Evidence & Sources
    </h3>
    <p class="mt-1 text-xs text-gray-500">Articles that support this content</p>

    <div class="mt-4 space-y-2">
        @foreach($evidence as $article)
            @php
                $path = str_replace('/superbigshit/articles/', '', $article->path);
                $weight = ($article->weight ?? 0.5) * 100;
            @endphp
            <a href="{{ route('articles.show', ['path' => $path]) }}"
               class="block rounded-lg border border-green-100 bg-green-50 p-3 transition-colors hover:bg-green-100">
                <div class="flex items-start gap-2">
                    <x-lucide-check-circle class="mt-0.5 h-4 w-4 flex-shrink-0 text-green-600" />
                    <div class="flex-1">
                        <p class="text-sm font-medium text-gray-900">
                            {{ $article->properties->title ?? $article->name }}
                        </p>
                        @if($weight > 0)
                            <div class="mt-2 flex items-center gap-2">
                                <div class="h-1.5 flex-1 rounded-full bg-green-200">
                                    <div class="h-full rounded-full bg-green-500" style="width: {{ $weight }}%"></div>
                                </div>
                                <span class="text-xs text-gray-500">{{ round($weight) }}%</span>
                            </div>
                        @endif
                    </div>
                </div>
            </a>
        @endforeach
    </div>
</div>
