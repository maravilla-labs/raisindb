import { LayoutDashboard, Inbox, Plus, FileAudio, Settings, User } from 'lucide-svelte';
import type { ComponentType } from 'svelte';

export interface NavItem {
	href: string;
	label: string;
	icon: ComponentType;
	badge?: 'inbox' | 'orders' | 'queue';
}

export interface NavSection {
	title: string;
	collapsible: boolean;
	defaultExpanded: boolean;
	items: NavItem[];
}

export const customerNav: NavSection[] = [
	{
		title: 'Dashboard',
		collapsible: true,
		defaultExpanded: true,
		items: [
			{ href: '/dashboard', label: 'Dashboard', icon: LayoutDashboard },
			{ href: '/dashboard', label: 'Inbox', icon: Inbox, badge: 'inbox' }
		]
	},
	{
		title: 'Orders',
		collapsible: true,
		defaultExpanded: true,
		items: [
			{ href: '/orders/new', label: 'New Order', icon: Plus },
			{ href: '/orders', label: 'All Orders', icon: FileAudio }
		]
	},
	{
		title: 'Account',
		collapsible: true,
		defaultExpanded: false,
		items: [
			{ href: '/settings', label: 'Settings', icon: Settings },
			{ href: '/profile', label: 'Profile', icon: User }
		]
	}
];
