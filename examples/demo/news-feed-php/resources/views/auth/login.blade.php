@extends('layouts.app')

@section('title', 'Login')

@section('content')
<div class="flex min-h-[60vh] items-center justify-center">
    <div class="w-full max-w-md">
        <div class="rounded-xl bg-white p-8 shadow-lg">
            {{-- Header --}}
            <div class="mb-8 text-center">
                <div class="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-blue-100">
                    <x-lucide-log-in class="h-6 w-6 text-blue-600" />
                </div>
                <h1 class="text-2xl font-bold text-gray-900">Welcome back</h1>
                <p class="mt-2 text-gray-600">Sign in to your account</p>
            </div>

            {{-- Error Messages --}}
            @if ($errors->any())
                <div class="mb-6 rounded-lg bg-red-50 p-4 text-sm text-red-700">
                    <div class="flex items-center gap-2">
                        <x-lucide-alert-circle class="h-5 w-5 flex-shrink-0" />
                        <span>{{ $errors->first() }}</span>
                    </div>
                </div>
            @endif

            {{-- Login Form --}}
            <form method="POST" action="{{ route('auth.login') }}" class="space-y-6">
                @csrf
                <input type="hidden" name="redirect" value="{{ $redirect }}">

                {{-- Email --}}
                <div>
                    <label for="email" class="block text-sm font-medium text-gray-700">Email address</label>
                    <div class="relative mt-1">
                        <div class="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3">
                            <x-lucide-mail class="h-5 w-5 text-gray-400" />
                        </div>
                        <input
                            type="email"
                            id="email"
                            name="email"
                            value="{{ old('email') }}"
                            required
                            autocomplete="email"
                            class="block w-full rounded-lg border border-gray-300 py-2.5 pl-10 pr-4 text-gray-900 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                            placeholder="you@example.com"
                        >
                    </div>
                </div>

                {{-- Password --}}
                <div>
                    <label for="password" class="block text-sm font-medium text-gray-700">Password</label>
                    <div class="relative mt-1">
                        <div class="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3">
                            <x-lucide-lock class="h-5 w-5 text-gray-400" />
                        </div>
                        <input
                            type="password"
                            id="password"
                            name="password"
                            required
                            autocomplete="current-password"
                            class="block w-full rounded-lg border border-gray-300 py-2.5 pl-10 pr-4 text-gray-900 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                            placeholder="Enter your password"
                        >
                    </div>
                </div>

                {{-- Remember Me --}}
                <div class="flex items-center">
                    <input
                        type="checkbox"
                        id="remember_me"
                        name="remember_me"
                        class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                    >
                    <label for="remember_me" class="ml-2 text-sm text-gray-600">Remember me</label>
                </div>

                {{-- Submit Button --}}
                <button
                    type="submit"
                    class="flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2"
                >
                    <x-lucide-log-in class="h-4 w-4" />
                    Sign in
                </button>
            </form>

            {{-- Register Link --}}
            <p class="mt-6 text-center text-sm text-gray-600">
                Don't have an account?
                <a href="{{ route('auth.register', ['redirect' => $redirect]) }}" class="font-medium text-blue-600 hover:text-blue-500">
                    Create one
                </a>
            </p>
        </div>
    </div>
</div>
@endsection
