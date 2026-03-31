<script lang="ts">
	import type { Component } from 'svelte';
	import { Search } from 'lucide-svelte';
	import * as icons from 'lucide-svelte';

	interface Props {
		value: string;
		onselect?: (icon: string) => void;
	}

	let { value = $bindable(''), onselect }: Props = $props();

	let searchQuery = $state('');
	let isOpen = $state(false);

	type IconProps = { size?: number; class?: string };

	// Common icons for tags - a curated subset
	const commonIcons = [
		'tag',
		'hash',
		'folder',
		'file',
		'star',
		'heart',
		'bookmark',
		'flag',
		'bell',
		'zap',
		'flame',
		'sparkles',
		'award',
		'trophy',
		'target',
		'crosshair',
		'compass',
		'map-pin',
		'globe',
		'home',
		'building',
		'users',
		'user',
		'person-standing',
		'briefcase',
		'shopping-cart',
		'shopping-bag',
		'package',
		'box',
		'gift',
		'calendar',
		'clock',
		'timer',
		'hourglass',
		'sun',
		'moon',
		'cloud',
		'umbrella',
		'code',
		'terminal',
		'cpu',
		'cog',
		'settings',
		'wrench',
		'hammer',
		'bug',
		'shield',
		'lock',
		'key',
		'eye',
		'camera',
		'image',
		'video',
		'music',
		'headphones',
		'mic',
		'phone',
		'mail',
		'message-circle',
		'send',
		'share',
		'link',
		'paperclip',
		'scissors',
		'pen',
		'pencil',
		'brush',
		'palette',
		'droplet',
		'leaf',
		'tree',
		'flower',
		'apple',
		'pizza',
		'coffee',
		'wine',
		'rocket',
		'plane',
		'car',
		'bike',
		'ship',
		'train',
		'book',
		'book-open',
		'graduation-cap',
		'lightbulb',
		'lamp',
		'battery',
		'wifi',
		'bluetooth',
		'database',
		'server',
		'hard-drive',
		'monitor',
		'smartphone',
		'tablet',
		'laptop',
		'keyboard',
		'mouse'
	];

	// Filter icons based on search
	const filteredIcons = $derived.by(() => {
		if (!searchQuery.trim()) return commonIcons;
		const query = searchQuery.toLowerCase();
		return commonIcons.filter((icon) => icon.includes(query));
	});

	// Convert icon name to PascalCase for lucide-svelte
	function getIconComponent(iconName: string): Component<IconProps> | null {
		const pascalName = iconName
			.split('-')
			.map((part) => part.charAt(0).toUpperCase() + part.slice(1))
			.join('');
		const icon = (icons as unknown as Record<string, Component<IconProps>>)[pascalName];
		return icon ?? null;
	}

	function selectIcon(iconName: string) {
		value = iconName;
		onselect?.(iconName);
		isOpen = false;
		searchQuery = '';
	}

	const SelectedIconComponent = $derived(value ? getIconComponent(value) : null);
</script>

<div class="relative">
	<button
		type="button"
		onclick={() => (isOpen = !isOpen)}
		class="flex h-10 w-full items-center gap-2 rounded-lg border border-gray-300 px-3 text-sm hover:border-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
	>
		{#if SelectedIconComponent}
			<SelectedIconComponent size={18} class="text-gray-700" />
			<span class="flex-1 text-left text-gray-700">{value}</span>
		{:else}
			<span class="flex-1 text-left text-gray-400">Select an icon...</span>
		{/if}
	</button>

	{#if isOpen}
		<div
			class="absolute z-20 mt-1 w-72 rounded-lg border border-gray-200 bg-white p-3 shadow-lg"
		>
			<!-- Search -->
			<div class="relative mb-3">
				<Search size={14} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400" />
				<input
					type="text"
					bind:value={searchQuery}
					placeholder="Search icons..."
					class="w-full rounded border border-gray-300 py-1.5 pl-8 pr-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
				/>
			</div>

			<!-- Icon grid -->
			<div class="grid max-h-48 grid-cols-8 gap-1 overflow-y-auto">
				{#each filteredIcons as iconName}
					{@const IconComponent = getIconComponent(iconName)}
					{#if IconComponent}
						<button
							type="button"
							onclick={() => selectIcon(iconName)}
							class="flex h-8 w-8 items-center justify-center rounded transition-colors {value ===
							iconName
								? 'bg-blue-100 text-blue-600'
								: 'hover:bg-gray-100 text-gray-600'}"
							title={iconName}
						>
							<IconComponent size={16} />
						</button>
					{/if}
				{/each}
			</div>

			{#if filteredIcons.length === 0}
				<p class="py-4 text-center text-sm text-gray-500">No icons found</p>
			{/if}

			<!-- Clear button -->
			{#if value}
				<button
					type="button"
					onclick={() => selectIcon('')}
					class="mt-2 w-full rounded border border-gray-200 py-1.5 text-sm text-gray-500 hover:bg-gray-50"
				>
					Clear selection
				</button>
			{/if}
		</div>
	{/if}
</div>
