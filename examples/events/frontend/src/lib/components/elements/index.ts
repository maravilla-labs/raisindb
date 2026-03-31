import type { Component } from 'svelte';
import type { ContentElement } from '$lib/types';
import HeroBlock from './HeroBlock.svelte';
import TextBlock from './TextBlock.svelte';
import EventListBlock from './EventListBlock.svelte';
import VenueListBlock from './VenueListBlock.svelte';
import SpeakerListBlock from './SpeakerListBlock.svelte';
import ScheduleBlock from './ScheduleBlock.svelte';

export const elementComponents: Record<string, Component<{ element: ContentElement }>> = {
	'events:HeroBlock': HeroBlock,
	'events:TextBlock': TextBlock,
	'events:EventListBlock': EventListBlock,
	'events:VenueListBlock': VenueListBlock,
	'events:SpeakerListBlock': SpeakerListBlock,
	'events:ScheduleBlock': ScheduleBlock,
};
