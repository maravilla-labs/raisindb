@extends('layouts.app')

@section('title', 'Register')

@section('content')
<div class="flex min-h-[60vh] items-center justify-center">
    <div class="w-full max-w-md">
        <div class="rounded-xl bg-white p-8 shadow-lg">
            {{-- Header --}}
            <div class="mb-8 text-center">
                <div class="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-green-100">
                    <x-lucide-user-plus class="h-6 w-6 text-green-600" />
                </div>
                <h1 class="text-2xl font-bold text-gray-900">Create an account</h1>
                <p class="mt-2 text-gray-600">Join the news feed community</p>
            </div>

            {{-- Error Messages --}}
            @if ($errors->any())
                <div class="mb-6 rounded-lg bg-red-50 p-4 text-sm text-red-700">
                    <ul class="list-inside list-disc space-y-1">
                        @foreach ($errors->all() as $error)
                            <li>{{ $error }}</li>
                        @endforeach
                    </ul>
                </div>
            @endif

            {{-- Register Form --}}
            <form method="POST" action="{{ route('auth.register') }}" class="space-y-6">
                @csrf
                <input type="hidden" name="redirect" value="{{ $redirect }}">

                {{-- Display Name --}}
                <div>
                    <label for="display_name" class="block text-sm font-medium text-gray-700">Display name <span class="text-gray-400">(optional)</span></label>
                    <div class="relative mt-1">
                        <div class="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3">
                            <x-lucide-user class="h-5 w-5 text-gray-400" />
                        </div>
                        <input
                            type="text"
                            id="display_name"
                            name="display_name"
                            value="{{ old('display_name') }}"
                            autocomplete="name"
                            class="block w-full rounded-lg border border-gray-300 py-2.5 pl-10 pr-4 text-gray-900 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                            placeholder="John Doe"
                        >
                    </div>
                </div>

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
                            autocomplete="new-password"
                            minlength="8"
                            class="block w-full rounded-lg border border-gray-300 py-2.5 pl-10 pr-4 text-gray-900 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                            placeholder="At least 8 characters"
                        >
                    </div>
                    <p class="mt-1 text-xs text-gray-500">Must be at least 8 characters</p>
                </div>

                {{-- Confirm Password --}}
                <div>
                    <label for="password_confirmation" class="block text-sm font-medium text-gray-700">Confirm password</label>
                    <div class="relative mt-1">
                        <div class="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3">
                            <x-lucide-lock class="h-5 w-5 text-gray-400" />
                        </div>
                        <input
                            type="password"
                            id="password_confirmation"
                            name="password_confirmation"
                            required
                            autocomplete="new-password"
                            class="block w-full rounded-lg border border-gray-300 py-2.5 pl-10 pr-4 text-gray-900 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                            placeholder="Confirm your password"
                        >
                    </div>
                </div>

                {{-- Submit Button --}}
                <button
                    type="submit"
                    class="flex w-full items-center justify-center gap-2 rounded-lg bg-green-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-green-700 focus:outline-none focus:ring-2 focus:ring-green-500 focus:ring-offset-2"
                >
                    <x-lucide-user-plus class="h-4 w-4" />
                    Create account
                </button>
            </form>

            {{-- Login Link --}}
            <p class="mt-6 text-center text-sm text-gray-600">
                Already have an account?
                <a href="{{ route('auth.login', ['redirect' => $redirect]) }}" class="font-medium text-blue-600 hover:text-blue-500">
                    Sign in
                </a>
            </p>
        </div>
    </div>
</div>
@endsection
