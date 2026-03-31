<script lang="ts">
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import {
		ArrowLeft,
		FileAudio,
		Image,
		User,
		Calendar,
		AlertTriangle,
		Sparkles,
		CheckCircle,
		Save
	} from 'lucide-svelte';
	import { Button } from '$lib/components/shared';
	import { AppHeader } from '$lib/components/layout';
	import { TranscriptEditor } from '$lib/components/transcript';
	import {
		orders,
		updateTranscriptBlocks,
		approveTranscript,
		type TranscriptBlock,
		type OutputDocumentType,
		type TemplateType
	} from '$lib/stores/orders';
	import { toasts } from '$lib/stores/toast';
	import { formatDateTime, formatFileSize, formatDocumentType } from '$lib/utils/formatters';

	let orderId = $derived($page.params.id);
	let order = $derived($orders.find((o) => o.id === orderId));

	// Local state for editing
	let blocks = $state<TranscriptBlock[]>([]);
	let outputDocumentType = $state<OutputDocumentType>('arztbrief');
	let templateUsed = $state<TemplateType>('swiss');
	let reviewNotes = $state('');
	let saving = $state(false);
	let approving = $state(false);

	// Initialize from order when it loads
	$effect(() => {
		if (order?.transcript) {
			blocks = [...order.transcript.blocks];
			outputDocumentType = order.transcript.outputDocumentType;
			templateUsed = order.transcript.templateUsed;
		}
	});

	function handleBlocksChange(newBlocks: TranscriptBlock[]) {
		blocks = newBlocks;
	}

	function handleDocumentTypeChange(type: OutputDocumentType) {
		outputDocumentType = type;
	}

	function handleTemplateChange(template: TemplateType) {
		templateUsed = template;
	}

	async function handleSave() {
		if (!order) return;
		saving = true;
		await new Promise((r) => setTimeout(r, 500));
		updateTranscriptBlocks(order.id, blocks, outputDocumentType, templateUsed, reviewNotes || undefined);
		toasts.success('Änderungen gespeichert');
		saving = false;
	}

	async function handleApprove() {
		if (!order) return;
		approving = true;
		await new Promise((r) => setTimeout(r, 500));

		// Save any pending changes first
		updateTranscriptBlocks(order.id, blocks, outputDocumentType, templateUsed, reviewNotes || undefined);

		approveTranscript(order.id);
		toasts.success('Transkript freigegeben und an Kunden gesendet');
		goto('/nurse/dashboard');
	}
</script>

<svelte:head>
	<title>Review {order?.orderNumber || 'Order'} - Pocket Medico</title>
</svelte:head>

<AppHeader title="Review Transcript">
	<a href="/nurse/dashboard">
		<Button variant="ghost">
			<ArrowLeft class="h-4 w-4" />
			Back to Queue
		</Button>
	</a>
</AppHeader>

{#if !order}
	<div class="flex h-96 items-center justify-center">
		<div class="text-center">
			<p class="text-lg font-medium text-gray-900">Order not found</p>
			<a href="/nurse/dashboard" class="mt-2 text-sm text-blue-600 hover:text-blue-700">
				Back to dashboard
			</a>
		</div>
	</div>
{:else}
	<div class="p-6">
		<div class="grid gap-6 lg:grid-cols-3">
			<!-- Left: Order Info -->
			<div class="space-y-6">
				<!-- Order Details Card -->
				<div class="rounded-xl border border-gray-200 bg-white p-5">
					<h2 class="mb-4 font-semibold text-gray-900">Order Details</h2>

					<div class="space-y-3">
						<div class="flex items-center justify-between">
							<span class="text-sm text-gray-500">Order Number</span>
							<span class="text-sm font-medium text-gray-900">{order.orderNumber}</span>
						</div>
						<div class="flex items-center justify-between">
							<span class="text-sm text-gray-500">Customer</span>
							<span class="text-sm font-medium text-gray-900">{order.customerName}</span>
						</div>
						<div class="flex items-center justify-between">
							<span class="text-sm text-gray-500">Patient</span>
							<span class="text-sm font-medium text-gray-900">{order.patientInitials}</span>
						</div>
						<div class="flex items-center justify-between">
							<span class="text-sm text-gray-500">Document Type</span>
							<span class="text-sm font-medium text-gray-900">{formatDocumentType(order.documentType)}</span>
						</div>
						<div class="flex items-center justify-between">
							<span class="text-sm text-gray-500">Priority</span>
							{#if order.priority === 'urgent'}
								<span class="flex items-center gap-1 text-sm font-medium text-red-600">
									<AlertTriangle class="h-4 w-4" />
									Urgent
								</span>
							{:else}
								<span class="text-sm font-medium text-gray-900">Normal</span>
							{/if}
						</div>
						<div class="flex items-center justify-between">
							<span class="text-sm text-gray-500">Created</span>
							<span class="text-sm text-gray-900">{formatDateTime(order.createdAt)}</span>
						</div>
						{#if order.aiConfidenceScore}
							<div class="flex items-center justify-between">
								<span class="text-sm text-gray-500">AI Confidence</span>
								<span class="flex items-center gap-1 text-sm font-medium
									{order.aiConfidenceScore >= 90 ? 'text-green-600' : order.aiConfidenceScore >= 80 ? 'text-yellow-600' : 'text-red-600'}">
									<Sparkles class="h-4 w-4" />
									{order.aiConfidenceScore}%
								</span>
							</div>
						{/if}
					</div>
				</div>

				<!-- Files Card -->
				<div class="rounded-xl border border-gray-200 bg-white p-5">
					<h2 class="mb-4 font-semibold text-gray-900">Source Files</h2>

					{#if order.files.length === 0}
						<p class="text-sm text-gray-500">No files attached</p>
					{:else}
						<div class="space-y-2">
							{#each order.files as file (file.id)}
								<div class="flex items-center gap-3 rounded-lg bg-gray-50 p-3">
									{#if file.type === 'audio'}
										<FileAudio class="h-5 w-5 text-blue-500" />
									{:else}
										<Image class="h-5 w-5 text-green-500" />
									{/if}
									<div class="min-w-0 flex-1">
										<p class="truncate text-sm font-medium text-gray-900">{file.name}</p>
										<p class="text-xs text-gray-500">{formatFileSize(file.size)}</p>
									</div>
								</div>
							{/each}
						</div>
					{/if}
				</div>

				{#if order.notes}
					<div class="rounded-xl border border-gray-200 bg-white p-5">
						<h2 class="mb-2 font-semibold text-gray-900">Customer Notes</h2>
						<p class="text-sm text-gray-600">{order.notes}</p>
					</div>
				{/if}
			</div>

			<!-- Right: Transcript Editor -->
			<div class="lg:col-span-2 space-y-4">
				{#if order.transcript}
					<TranscriptEditor
						{blocks}
						{outputDocumentType}
						{templateUsed}
						onBlocksChange={handleBlocksChange}
						onDocumentTypeChange={handleDocumentTypeChange}
						onTemplateChange={handleTemplateChange}
						onSave={handleSave}
						onApprove={handleApprove}
						{saving}
						{approving}
					/>
				{:else}
					<div class="rounded-xl border border-gray-200 bg-white p-8 text-center">
						<p class="text-sm text-gray-500">Kein Transkript verfügbar.</p>
					</div>
				{/if}

				<!-- Review Notes -->
				<div class="rounded-xl border border-gray-200 bg-white p-5">
					<h2 class="mb-2 font-semibold text-gray-900">Prüfungsnotizen (Optional)</h2>
					<textarea
						bind:value={reviewNotes}
						class="min-h-24 w-full rounded-lg border border-gray-300 p-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
						placeholder="Notizen zur Prüfung hinzufügen..."
					></textarea>
				</div>
			</div>
		</div>
	</div>
{/if}
