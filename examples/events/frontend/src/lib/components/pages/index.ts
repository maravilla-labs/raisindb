import type { Component } from 'svelte';
import type { Page } from '$lib/types';
import LandingPage from './LandingPage.svelte';
import ContentPage from './ContentPage.svelte';

export const pageComponents: Record<string, Component<{ page: Page }>> = {
	'events:LandingPage': LandingPage,
	'events:ContentPage': ContentPage,
};

export const defaultPageComponent = ContentPage;
