<form action="{{ route('search') }}" method="GET" class="relative">
    <x-lucide-search class="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-400" />
    <input type="text"
           name="q"
           value="{{ request('q') }}"
           placeholder="Search articles..."
           class="w-full rounded-lg border border-gray-300 bg-gray-50 py-2 pl-10 pr-4 text-sm transition-colors focus:border-blue-500 focus:bg-white focus:outline-none focus:ring-1 focus:ring-blue-500">
</form>
