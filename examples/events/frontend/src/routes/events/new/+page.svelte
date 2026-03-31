<script lang="ts">
	import { goto } from '$app/navigation';
	import { getWorkspace } from '$lib/raisin';

	let title = $state('');
	let slug = $state('');
	let description = $state('');
	let startDate = $state('');
	let endDate = $state('');
	let location = $state('');
	let category = $state('');
	let capacity = $state<number | undefined>(undefined);
	let price = $state(0);
	let featured = $state(false);
	let saving = $state(false);
	let error = $state('');

	function generateSlug(text: string): string {
		return text
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, '-')
			.replace(/^-|-$/g, '');
	}

	function handleTitleInput() {
		if (!slug || slug === generateSlug(title.slice(0, -1))) {
			slug = generateSlug(title);
		}
	}

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		if (!title || !slug || !startDate) {
			error = 'Title, slug, and start date are required.';
			return;
		}

		saving = true;
		error = '';

		try {
			const ws = await getWorkspace();
			const nodes = ws.nodes();
			await nodes.create({
				type: 'events:Event',
				path: `/events/${slug}`,
				properties: {
					title,
					slug,
					description: description || undefined,
					start_date: startDate,
					end_date: endDate || undefined,
					location: location || undefined,
					category: category || undefined,
					capacity: capacity || undefined,
					price,
					featured,
					status: 'draft',
				},
			});
			goto(`/events/${slug}`);
		} catch (err) {
			error = err instanceof Error ? err.message : 'Failed to create event';
			saving = false;
		}
	}
</script>

<svelte:head>
	<title>Submit Event | Keller Basel</title>
</svelte:head>

<div class="container">
	<div class="page-header">
		<a href="/events">&larr; Back to events</a>
		<h1>Submit Event</h1>
	</div>

	{#if error}
		<div class="error-banner">{error}</div>
	{/if}

	<form class="form" onsubmit={handleSubmit}>
		<div class="form-group">
			<label for="title">Title *</label>
			<input id="title" type="text" bind:value={title} oninput={handleTitleInput} required placeholder="Event name" />
		</div>

		<div class="form-group">
			<label for="slug">Slug *</label>
			<input id="slug" type="text" bind:value={slug} required placeholder="url-friendly-name" />
		</div>

		<div class="form-group">
			<label for="description">Description</label>
			<textarea id="description" bind:value={description} rows="4" placeholder="Tell us about this event..."></textarea>
		</div>

		<div class="form-row">
			<div class="form-group">
				<label for="startDate">Start Date *</label>
				<input id="startDate" type="datetime-local" bind:value={startDate} required />
			</div>
			<div class="form-group">
				<label for="endDate">End Date</label>
				<input id="endDate" type="datetime-local" bind:value={endDate} />
			</div>
		</div>

		<div class="form-group">
			<label for="location">Location</label>
			<input id="location" type="text" bind:value={location} placeholder="Venue name or address" />
		</div>

		<div class="form-row">
			<div class="form-group">
				<label for="category">Category</label>
				<input id="category" type="text" bind:value={category} placeholder="Techno, House, Experimental..." />
			</div>
			<div class="form-group">
				<label for="capacity">Capacity</label>
				<input id="capacity" type="number" bind:value={capacity} min="0" />
			</div>
		</div>

		<div class="form-row">
			<div class="form-group">
				<label for="price">Price (CHF)</label>
				<input id="price" type="number" bind:value={price} min="0" step="0.01" />
			</div>
			<div class="form-group">
				<label class="checkbox-label">
					<input type="checkbox" bind:checked={featured} />
					Featured Event
				</label>
			</div>
		</div>

		<div class="form-actions">
			<a href="/events" class="btn-secondary">Cancel</a>
			<button type="submit" class="btn-primary" disabled={saving}>
				{saving ? 'Submitting...' : 'Submit Event'}
			</button>
		</div>
	</form>
</div>

<style>
	.form {
		max-width: 640px;
		padding-bottom: var(--space-3xl);
	}

	.form-group {
		margin-bottom: var(--space-lg);
	}

	.form-group label {
		display: block;
		font-weight: 500;
		margin-bottom: var(--space-sm);
		color: var(--color-text-secondary);
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
	}

	.form-group input[type='text'],
	.form-group input[type='number'],
	.form-group input[type='datetime-local'],
	.form-group textarea {
		width: 100%;
		padding: 0.7rem 0.9rem;
		border: 1px solid var(--color-border);
		border-radius: var(--radius);
		font-family: var(--font-body);
		font-size: 0.95rem;
		background: var(--color-surface);
		color: var(--color-text);
		transition: border-color 0.2s;
	}

	.form-group input::placeholder,
	.form-group textarea::placeholder {
		color: var(--color-text-muted);
	}

	.form-group input:focus,
	.form-group textarea:focus {
		outline: none;
		border-color: var(--color-accent);
		box-shadow: 0 0 0 2px var(--color-accent-dim);
	}

	.form-row {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: var(--space-md);
	}

	.checkbox-label {
		display: flex !important;
		align-items: center;
		gap: var(--space-sm);
		cursor: pointer;
		padding-top: 1.75rem;
		text-transform: none !important;
		letter-spacing: 0 !important;
		font-size: 0.9rem !important;
		color: var(--color-text) !important;
	}

	.checkbox-label input[type='checkbox'] {
		width: 1.1rem;
		height: 1.1rem;
		accent-color: var(--color-accent);
	}

	.form-actions {
		display: flex;
		gap: var(--space-md);
		margin-top: var(--space-xl);
	}

	.error-banner {
		background: rgba(220, 38, 38, 0.1);
		color: #f87171;
		padding: 0.75rem 1rem;
		border-radius: var(--radius);
		margin-bottom: var(--space-lg);
		border: 1px solid rgba(220, 38, 38, 0.2);
		max-width: 640px;
		font-size: 0.9rem;
	}

	@media (max-width: 640px) {
		.form-row {
			grid-template-columns: 1fr;
		}
	}
</style>
