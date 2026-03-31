<script lang="ts">
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import {
		ArrowLeft,
		Download,
		FileAudio,
		FileImage,
		Clock,
		CheckCircle,
		Loader2,
		Eye,
		AlertCircle,
		Send,
		Plus
	} from 'lucide-svelte';
	import { Button } from '$lib/components/shared';
	import { AppHeader } from '$lib/components/layout';
	import { FileUploadZone, FileList, StatusBadge } from '$lib/components/orders';
	import { TranscriptViewer } from '$lib/components/transcript';
	import {
		orders,
		addFileToOrder,
		removeFileFromOrder,
		submitOrder,
		markAsDownloaded,
		type UploadedFile,
		type Order
	} from '$lib/stores/orders';
	import { toasts } from '$lib/stores/toast';
	import { formatDateTime, formatDocumentType, formatFileSize, formatDuration } from '$lib/utils/formatters';

	// Get order from store
	let order = $derived($orders.find((o) => o.id === $page.params.id));

	// Local state for adding files
	let pendingFiles = $state<UploadedFile[]>([]);
	let isSubmitting = $state(false);

	// Status timeline configuration
	const statusSteps = [
		{ status: 'pending', label: 'Order Created', icon: Clock },
		{ status: 'processing', label: 'Processing', icon: Loader2 },
		{ status: 'ai_complete', label: 'AI Complete', icon: CheckCircle },
		{ status: 'in_review', label: 'Nurse Review', icon: Eye },
		{ status: 'ready', label: 'Ready', icon: CheckCircle },
		{ status: 'downloaded', label: 'Downloaded', icon: Download }
	];

	// Status step index lookup
	const statusOrder = ['pending', 'processing', 'ai_complete', 'in_review', 'ready', 'downloaded'];

	function getStepIndex(status: string): number {
		return statusOrder.indexOf(status);
	}

	// Computed values for status timeline
	let currentIndex = $derived(order ? getStepIndex(order.status) : -1);
	let filteredSteps = $derived(
		order?.tier === 'light'
			? statusSteps.filter((s) => !['ai_complete', 'in_review'].includes(s.status))
			: statusSteps
	);

	// Handle file uploads (for pending orders)
	function handleFiles(newFiles: File[]) {
		const uploadedFiles: UploadedFile[] = newFiles.map((file) => ({
			id: crypto.randomUUID(),
			name: file.name,
			type: file.type.startsWith('audio/') ? 'audio' : 'image',
			size: file.size,
			url: URL.createObjectURL(file)
		}));
		pendingFiles = [...pendingFiles, ...uploadedFiles];
	}

	// Handle pending file removal
	function handleRemovePendingFile(fileId: string) {
		const file = pendingFiles.find((f) => f.id === fileId);
		if (file) {
			URL.revokeObjectURL(file.url);
		}
		pendingFiles = pendingFiles.filter((f) => f.id !== fileId);
	}

	// Submit additional files
	async function handleSubmitFiles() {
		if (!order || pendingFiles.length === 0 || isSubmitting) return;

		isSubmitting = true;

		try {
			// Add files to the order
			for (const file of pendingFiles) {
				addFileToOrder(order.id, file);
			}

			// Submit the order for processing
			submitOrder(order.id);

			pendingFiles = [];
			toasts.success('Files added and order submitted!');
		} catch (error) {
			toasts.error('Failed to submit files. Please try again.');
		} finally {
			isSubmitting = false;
		}
	}

	// Handle transcript download (legacy - TranscriptViewer now handles this)
	function handleDownload() {
		if (!order?.transcript) return;

		// Generate content from blocks
		let content = '';
		for (const block of order.transcript.blocks) {
			content += `<h3>${block.label}</h3>\n`;
			content += `<p>${block.content.replace(/\n/g, '<br>')}</p>\n\n`;
		}

		// Create a blob with the transcript content
		const blob = new Blob([content], { type: 'text/html' });
		const url = URL.createObjectURL(blob);

		// Create download link
		const a = document.createElement('a');
		a.href = url;
		a.download = `${order.orderNumber}-transcript.html`;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);

		URL.revokeObjectURL(url);

		// Mark as downloaded
		markAsDownloaded(order.id);
		toasts.success('Transkript heruntergeladen!');
	}
</script>

<svelte:head>
	<title>{order ? `Order ${order.orderNumber}` : 'Order'} - Pocket Medico</title>
</svelte:head>

<AppHeader title={order ? `Order ${order.orderNumber}` : 'Order Details'}>
	<a href="/orders">
		<Button variant="ghost">
			<ArrowLeft class="h-4 w-4" />
			Back to Orders
		</Button>
	</a>
</AppHeader>

<div class="p-6">
	{#if !order}
		<div class="rounded-xl border border-gray-200 bg-white p-8 text-center">
			<AlertCircle class="mx-auto mb-4 h-12 w-12 text-gray-400" />
			<h2 class="text-lg font-semibold text-gray-900">Order not found</h2>
			<p class="mt-2 text-gray-500">This order may have been deleted or the link is invalid.</p>
			<a href="/orders" class="mt-4 inline-block">
				<Button>View All Orders</Button>
			</a>
		</div>
	{:else}
		<div class="mx-auto max-w-4xl space-y-6">
			<!-- Order Header -->
			<div class="rounded-xl border border-gray-200 bg-white p-6">
				<div class="flex flex-wrap items-start justify-between gap-4">
					<div>
						<div class="flex items-center gap-3">
							<h1 class="text-xl font-semibold text-gray-900">{order.orderNumber}</h1>
							<StatusBadge status={order.status} />
						</div>
						<p class="mt-1 text-sm text-gray-500">
							Patient: {order.patientInitials} | {formatDocumentType(order.documentType)}
						</p>
					</div>

					<div class="flex items-center gap-2">
						<span
							class="rounded-lg px-3 py-1.5 text-sm font-medium
								{order.tier === 'pro'
								? 'bg-purple-100 text-purple-700'
								: 'bg-gray-100 text-gray-700'}"
						>
							{order.tier === 'pro' ? 'Pro' : 'Light'}
						</span>
						{#if order.priority === 'urgent'}
							<span class="rounded-lg bg-orange-100 px-3 py-1.5 text-sm font-medium text-orange-700">
								Urgent
							</span>
						{/if}
					</div>
				</div>

				<div class="mt-4 grid gap-4 text-sm sm:grid-cols-3">
					<div>
						<p class="text-gray-500">Created</p>
						<p class="font-medium text-gray-900">{formatDateTime(order.createdAt)}</p>
					</div>
					<div>
						<p class="text-gray-500">Last Updated</p>
						<p class="font-medium text-gray-900">{formatDateTime(order.updatedAt)}</p>
					</div>
					{#if order.completedAt}
						<div>
							<p class="text-gray-500">Completed</p>
							<p class="font-medium text-gray-900">{formatDateTime(order.completedAt)}</p>
						</div>
					{/if}
				</div>

				{#if order.notes}
					<div class="mt-4 rounded-lg bg-gray-50 p-3">
						<p class="text-xs font-medium text-gray-500">Notes</p>
						<p class="mt-1 text-sm text-gray-700">{order.notes}</p>
					</div>
				{/if}
			</div>

			<!-- Status Timeline -->
			<div class="rounded-xl border border-gray-200 bg-white p-6">
				<h2 class="mb-4 text-lg font-semibold text-gray-900">Status Timeline</h2>

				<div class="flex items-center justify-between">
					{#each filteredSteps as step, index (step.status)}
						{@const stepIndex = getStepIndex(step.status)}
						{@const isCompleted = stepIndex <= currentIndex}
						{@const isCurrent = step.status === order.status}

						<div class="flex flex-col items-center">
							<div
								class="flex h-10 w-10 items-center justify-center rounded-full
									{isCompleted
									? isCurrent
										? 'bg-blue-600 text-white'
										: 'bg-green-600 text-white'
									: 'bg-gray-100 text-gray-400'}"
							>
								<svelte:component this={step.icon} class="h-5 w-5" />
							</div>
							<p
								class="mt-2 text-center text-xs font-medium
									{isCompleted ? 'text-gray-900' : 'text-gray-400'}"
							>
								{step.label}
							</p>
						</div>

						{#if index < filteredSteps.length - 1}
							<div
								class="flex-1 border-t-2 mx-2
									{stepIndex < currentIndex ? 'border-green-600' : 'border-gray-200'}"
							></div>
						{/if}
					{/each}
				</div>

				{#if order.status === 'error'}
					<div class="mt-4 flex items-center gap-2 rounded-lg bg-red-50 p-3 text-sm text-red-700">
						<AlertCircle class="h-4 w-4" />
						<span>An error occurred during processing. Please contact support.</span>
					</div>
				{/if}
			</div>

			<!-- Files Section -->
			<div class="rounded-xl border border-gray-200 bg-white p-6">
				<h2 class="mb-4 text-lg font-semibold text-gray-900">Files</h2>

				{#if order.files.length > 0}
					<ul class="divide-y divide-gray-100 rounded-lg border border-gray-200">
						{#each order.files as file (file.id)}
							<li class="flex items-center gap-3 p-3">
								<div class="flex-shrink-0">
									{#if file.type === 'audio'}
										<FileAudio class="h-5 w-5 text-blue-500" />
									{:else}
										<FileImage class="h-5 w-5 text-green-500" />
									{/if}
								</div>
								<div class="min-w-0 flex-1">
									<p class="truncate text-sm font-medium text-gray-900">{file.name}</p>
									<p class="text-xs text-gray-500">
										{formatFileSize(file.size)}
										{#if file.durationSeconds}
											| {formatDuration(file.durationSeconds)}
										{/if}
									</p>
								</div>
							</li>
						{/each}
					</ul>
				{:else}
					<p class="text-sm text-gray-500">No files uploaded yet.</p>
				{/if}

				<!-- Add more files for pending orders -->
				{#if order.status === 'pending'}
					<div class="mt-4 space-y-3">
						<p class="text-sm text-gray-600">Add files to your order:</p>
						<FileUploadZone onfiles={handleFiles} />
						<FileList files={pendingFiles} onremove={handleRemovePendingFile} />

						{#if pendingFiles.length > 0 || order.files.length > 0}
							<div class="flex justify-end">
								<Button
									onclick={handleSubmitFiles}
									disabled={order.files.length === 0 && pendingFiles.length === 0}
									loading={isSubmitting}
								>
									<Send class="h-4 w-4" />
									Submit Order
								</Button>
							</div>
						{/if}
					</div>
				{/if}
			</div>

			<!-- Transcript Section -->
			{#if order.transcript && (order.status === 'ready' || order.status === 'downloaded')}
				<TranscriptViewer
					transcript={order.transcript}
					patientInitials={order.patientInitials}
					orderNumber={order.orderNumber}
				/>
			{:else if order.status === 'processing'}
				<div class="rounded-xl border border-gray-200 bg-white p-6">
					<div class="flex items-center gap-3">
						<Loader2 class="h-6 w-6 animate-spin text-blue-600" />
						<div>
							<h2 class="text-lg font-semibold text-gray-900">Processing...</h2>
							<p class="text-sm text-gray-500">Your transcript is being generated by our AI.</p>
						</div>
					</div>
				</div>
			{:else if order.status === 'ai_complete' || order.status === 'in_review'}
				<div class="rounded-xl border border-gray-200 bg-white p-6">
					<div class="flex items-center gap-3">
						<Eye class="h-6 w-6 text-blue-600" />
						<div>
							<h2 class="text-lg font-semibold text-gray-900">Under Review</h2>
							<p class="text-sm text-gray-500">
								Your transcript is being reviewed by a medical professional.
							</p>
						</div>
					</div>
				</div>
			{/if}
		</div>
	{/if}
</div>
