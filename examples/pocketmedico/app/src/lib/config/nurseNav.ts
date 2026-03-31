import { LayoutDashboard, Inbox, ClipboardList, CheckSquare, Settings, User } from 'lucide-svelte';
import type { NavSection } from './customerNav';

export const nurseNav: NavSection[] = [
	{
		title: 'Dashboard',
		collapsible: true,
		defaultExpanded: true,
		items: [
			{ href: '/nurse/dashboard', label: 'Dashboard', icon: LayoutDashboard },
			{ href: '/nurse/dashboard', label: 'Inbox', icon: Inbox, badge: 'inbox' }
		]
	},
	{
		title: 'Review',
		collapsible: true,
		defaultExpanded: true,
		items: [
			{ href: '/nurse/dashboard', label: 'Queue', icon: ClipboardList, badge: 'queue' },
			{ href: '/nurse/completed', label: 'Completed', icon: CheckSquare }
		]
	},
	{
		title: 'Account',
		collapsible: true,
		defaultExpanded: false,
		items: [
			{ href: '/nurse/settings', label: 'Settings', icon: Settings },
			{ href: '/nurse/profile', label: 'Profile', icon: User }
		]
	}
];
