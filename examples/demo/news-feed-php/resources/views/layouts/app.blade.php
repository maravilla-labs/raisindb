<!DOCTYPE html>
<html lang="{{ str_replace('_', '-', app()->getLocale()) }}">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta name="csrf-token" content="{{ csrf_token() }}">
    <title>@yield('title', 'News Feed') - RaisinDB Demo</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
    @vite(['resources/css/app.css', 'resources/js/app.js'])
</head>
<body class="min-h-screen bg-gray-50">
    <div x-data="toastManager()" x-on:toast.window="show($event.detail.type, $event.detail.message)">
        {{-- Header --}}
        <header class="sticky top-0 z-40 border-b border-gray-200 bg-white shadow-sm">
            <div class="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
                <div class="flex h-16 items-center justify-between">
                    {{-- Logo --}}
                    <a href="{{ route('home') }}" class="flex items-center gap-2">
                        <x-lucide-newspaper class="h-8 w-8 text-blue-600" />
                        <span class="text-xl font-bold text-gray-900">News Feed</span>
                    </a>

                    {{-- Search --}}
                    <div class="hidden w-96 md:block">
                        @include('components.search-input')
                    </div>

                    {{-- Actions --}}
                    <div class="flex items-center gap-2">
                        <a href="{{ route('settings.categories.index') }}"
                           class="rounded-lg p-2 text-gray-500 transition-colors hover:bg-gray-100 hover:text-gray-700"
                           title="Settings">
                            <x-lucide-settings class="h-5 w-5" />
                        </a>

                        @if($currentUser ?? null)
                            {{-- Logged in: Show user menu --}}
                            <a href="{{ route('articles.create') }}"
                               class="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-700">
                                <x-lucide-pen-square class="h-4 w-4" />
                                <span class="hidden sm:inline">New Article</span>
                            </a>

                            <div x-data="{ open: false }" class="relative">
                                <button @click="open = !open"
                                        class="flex items-center gap-2 rounded-lg p-2 text-gray-700 transition-colors hover:bg-gray-100">
                                    <div class="flex h-8 w-8 items-center justify-center rounded-full bg-blue-100 text-blue-600">
                                        @if($currentUser['avatar_url'] ?? null)
                                            <img src="{{ $currentUser['avatar_url'] }}" alt="" class="h-8 w-8 rounded-full">
                                        @else
                                            <x-lucide-user class="h-4 w-4" />
                                        @endif
                                    </div>
                                    <span class="hidden max-w-[120px] truncate text-sm font-medium sm:block">
                                        {{ $currentUser['display_name'] ?? $currentUser['email'] ?? 'User' }}
                                    </span>
                                    <x-lucide-chevron-down class="h-4 w-4 text-gray-400" />
                                </button>

                                <div x-show="open"
                                     @click.away="open = false"
                                     x-transition
                                     class="absolute right-0 mt-2 w-48 rounded-lg border border-gray-200 bg-white py-1 shadow-lg">
                                    <div class="border-b border-gray-100 px-4 py-2">
                                        <p class="truncate text-sm font-medium text-gray-900">{{ $currentUser['display_name'] ?? 'User' }}</p>
                                        <p class="truncate text-xs text-gray-500">{{ $currentUser['email'] ?? '' }}</p>
                                    </div>
                                    <form method="POST" action="{{ route('auth.logout') }}">
                                        @csrf
                                        <button type="submit"
                                                class="flex w-full items-center gap-2 px-4 py-2 text-left text-sm text-gray-700 hover:bg-gray-100">
                                            <x-lucide-log-out class="h-4 w-4" />
                                            Sign out
                                        </button>
                                    </form>
                                </div>
                            </div>
                        @else
                            {{-- Not logged in: Show login/register buttons --}}
                            <a href="{{ route('auth.login') }}"
                               class="inline-flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-medium text-gray-700 transition-colors hover:bg-gray-100">
                                <x-lucide-log-in class="h-4 w-4" />
                                <span class="hidden sm:inline">Sign in</span>
                            </a>
                            <a href="{{ route('auth.register') }}"
                               class="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-700">
                                <x-lucide-user-plus class="h-4 w-4" />
                                <span class="hidden sm:inline">Register</span>
                            </a>
                        @endif
                    </div>
                </div>
            </div>
        </header>

        {{-- Category Tabs --}}
        @include('components.category-tabs', ['categories' => $categories ?? []])

        {{-- Main Content --}}
        <main class="mx-auto max-w-7xl px-4 py-8 sm:px-6 lg:px-8">
            {{-- Flash Messages --}}
            @if (session('success'))
                <div class="mb-6 rounded-lg bg-green-50 p-4 text-green-800">
                    <div class="flex items-center gap-2">
                        <x-lucide-check-circle class="h-5 w-5" />
                        {{ session('success') }}
                    </div>
                </div>
            @endif

            @if (session('error'))
                <div class="mb-6 rounded-lg bg-red-50 p-4 text-red-800">
                    <div class="flex items-center gap-2">
                        <x-lucide-x-circle class="h-5 w-5" />
                        {{ session('error') }}
                    </div>
                </div>
            @endif

            @yield('content')
        </main>

        {{-- Footer --}}
        <footer class="border-t border-gray-200 bg-white py-8">
            <div class="mx-auto max-w-7xl px-4 text-center text-sm text-gray-500 sm:px-6 lg:px-8">
                <p>
                    Powered by <a href="https://raisindb.com" class="text-blue-600 hover:underline">RaisinDB</a>
                    - A hierarchical PostgreSQL database
                </p>
            </div>
        </footer>

        {{-- Toast Notifications --}}
        @include('components.toast')
    </div>
</body>
</html>
