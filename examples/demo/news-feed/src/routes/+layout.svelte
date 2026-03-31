<script lang="ts">
	import '../app.css';
	import { Newspaper, PenSquare, Settings, LogIn, UserPlus, LogOut, User } from 'lucide-svelte';
	import CategoryTabs from '$lib/components/CategoryTabs.svelte';
	import SearchInput from '$lib/components/SearchInput.svelte';
	import Toast from '$lib/components/Toast.svelte';
	import PoolStats from '$lib/components/PoolStats.svelte';

	let { children, data } = $props();

	let showUserMenu = $state(false);
</script>

<svelte:head>
	<title>News Feed - RaisinDB Demo</title>
	<meta name="description" content="A demo news feed application powered by RaisinDB" />
</svelte:head>

<div class="min-h-screen bg-gray-50">
	<header class="sticky top-0 z-40 border-b border-gray-200 bg-white shadow-sm">
		<div class="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
			<div class="flex h-16 items-center justify-between">
				<a href="/" class="flex items-center gap-2">
					<Newspaper class="h-8 w-8 text-blue-600" />
					<span class="text-xl font-bold text-gray-900">News Feed</span>
				</a>

				<div class="hidden w-96 md:block">
					<SearchInput />
				</div>

				<div class="flex items-center gap-2">
					<a
						href="/settings/categories"
						class="rounded-lg p-2 text-gray-500 transition-colors hover:bg-gray-100 hover:text-gray-700"
						title="Settings"
					>
						<Settings class="h-5 w-5" />
					</a>

					{#if data.user}
						<!-- Logged in: show New Article button and user menu -->
						<a
							href="/articles/new"
							class="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-700"
						>
							<PenSquare class="h-4 w-4" />
							<span class="hidden sm:inline">New Article</span>
						</a>

						<div class="relative">
							<button
								type="button"
								onclick={() => (showUserMenu = !showUserMenu)}
								class="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-gray-700 transition-colors hover:bg-gray-100"
							>
								<div
									class="flex h-8 w-8 items-center justify-center rounded-full bg-blue-100 text-blue-600"
								>
									<User class="h-4 w-4" />
								</div>
								<span class="hidden max-w-[100px] truncate sm:block">
									{data.user.displayName || data.user.email}
								</span>
							</button>

							{#if showUserMenu}
								<div
									class="absolute right-0 z-50 mt-2 w-48 rounded-lg bg-white py-1 shadow-lg ring-1 ring-black ring-opacity-5"
								>
									<div class="border-b border-gray-100 px-4 py-2">
										<p class="truncate text-sm font-medium text-gray-900">
											{data.user.displayName || 'User'}
										</p>
										<p class="truncate text-xs text-gray-500">{data.user.email}</p>
									</div>
									<form method="POST" action="/auth/logout">
										<button
											type="submit"
											class="flex w-full items-center gap-2 px-4 py-2 text-sm text-gray-700 hover:bg-gray-100"
										>
											<LogOut class="h-4 w-4" />
											Sign out
										</button>
									</form>
								</div>
							{/if}
						</div>
					{:else}
						<!-- Not logged in: show login/register links -->
						<a
							href="/auth/login"
							class="inline-flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-gray-700 transition-colors hover:bg-gray-100"
						>
							<LogIn class="h-4 w-4" />
							<span class="hidden sm:inline">Sign in</span>
						</a>
						<a
							href="/auth/register"
							class="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-700"
						>
							<UserPlus class="h-4 w-4" />
							<span class="hidden sm:inline">Sign up</span>
						</a>
					{/if}
				</div>
			</div>
		</div>
	</header>

	<CategoryTabs categories={data.categories} />

	<main class="mx-auto max-w-7xl px-4 py-8 sm:px-6 lg:px-8">
		{@render children()}
	</main>

	<footer class="border-t border-gray-200 bg-white py-8">
		<div class="mx-auto max-w-7xl px-4 text-center text-sm text-gray-500 sm:px-6 lg:px-8">
			<p>
				Powered by <a href="https://raisindb.com" class="text-blue-600 hover:underline">RaisinDB</a>
				- A hierarchical PostgreSQL database
			</p>
		</div>
	</footer>
</div>

<Toast />
<PoolStats />
