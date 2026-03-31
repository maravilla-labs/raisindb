<script lang="ts">
	import { Download, FileText, CheckCircle, Clock, Printer } from 'lucide-svelte';
	import type { Transcript } from '$lib/stores/orders';
	import { documentTypeLabels, blockStyles } from '$lib/config/transcriptTemplates';
	import { Button } from '$lib/components/shared';

	interface Props {
		transcript: Transcript;
		patientInitials?: string;
		orderNumber?: string;
	}

	let { transcript, patientInitials, orderNumber }: Props = $props();

	const documentTypeLabel = $derived(documentTypeLabels[transcript.outputDocumentType] || transcript.outputDocumentType);

	function handlePrint() {
		window.print();
	}

	function handleDownload() {
		// Generate text content
		let content = `${documentTypeLabel.toUpperCase()}\n`;
		content += `${'='.repeat(40)}\n\n`;

		if (orderNumber) {
			content += `Auftragsnummer: ${orderNumber}\n`;
		}
		if (patientInitials) {
			content += `Patient: ${patientInitials}\n`;
		}
		content += `Erstellt: ${new Date().toLocaleDateString('de-DE')}\n\n`;
		content += `${'='.repeat(40)}\n\n`;

		for (const block of transcript.blocks) {
			content += `${block.label.toUpperCase()}\n`;
			content += `${'-'.repeat(block.label.length)}\n`;
			content += `${block.content}\n\n`;
		}

		if (transcript.humanReviewed && transcript.reviewedAt) {
			content += `\n${'='.repeat(40)}\n`;
			content += `Geprüft am: ${new Date(transcript.reviewedAt).toLocaleDateString('de-DE')}\n`;
		}

		// Create download
		const blob = new Blob([content], { type: 'text/plain;charset=utf-8' });
		const url = URL.createObjectURL(blob);
		const a = document.createElement('a');
		a.href = url;
		a.download = `${documentTypeLabel.toLowerCase().replace(/\s+/g, '-')}-${orderNumber || 'dokument'}.txt`;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
		URL.revokeObjectURL(url);
	}
</script>

<div class="rounded-lg border border-gray-200 bg-white shadow-sm print:border-0 print:shadow-none">
	<!-- Header -->
	<div class="border-b border-gray-200 bg-gray-50 px-6 py-4 print:bg-white">
		<div class="flex items-center justify-between">
			<div class="flex items-center gap-3">
				<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-blue-100">
					<FileText class="h-5 w-5 text-blue-600" />
				</div>
				<div>
					<h2 class="text-lg font-semibold text-gray-900">{documentTypeLabel}</h2>
					<div class="flex items-center gap-2 text-sm text-gray-500">
						{#if transcript.humanReviewed}
							<span class="inline-flex items-center gap-1 text-green-600">
								<CheckCircle class="h-3.5 w-3.5" />
								Geprüft
							</span>
						{:else if transcript.aiGenerated}
							<span class="inline-flex items-center gap-1 text-amber-600">
								<Clock class="h-3.5 w-3.5" />
								KI-generiert, Prüfung ausstehend
							</span>
						{/if}
						<span class="text-gray-300">|</span>
						<span>Version {transcript.version}</span>
					</div>
				</div>
			</div>

			<div class="flex items-center gap-2 print:hidden">
				<Button variant="secondary" onclick={handlePrint}>
					<Printer class="mr-1.5 h-4 w-4" />
					Drucken
				</Button>
				<Button onclick={handleDownload}>
					<Download class="mr-1.5 h-4 w-4" />
					Herunterladen
				</Button>
			</div>
		</div>
	</div>

	<!-- Content -->
	<div class="divide-y divide-gray-100 px-6 py-4">
		{#if transcript.blocks.length === 0}
			<div class="py-8 text-center text-gray-500">
				<FileText class="mx-auto h-12 w-12 text-gray-300" />
				<p class="mt-2">Kein Inhalt verfügbar</p>
			</div>
		{:else}
			{#each transcript.blocks as block (block.id)}
				{@const style = blockStyles[block.type] || blockStyles.notes}
				<div class="py-4 first:pt-0 last:pb-0">
					<div class="mb-2 flex items-center gap-2">
						<span
							class="rounded px-2 py-0.5 text-xs font-medium {style.headerBg} text-gray-700"
						>
							{block.label}
						</span>
					</div>
					<div class="whitespace-pre-wrap text-sm leading-relaxed text-gray-700">
						{block.content || '(Kein Inhalt)'}
					</div>
				</div>
			{/each}
		{/if}
	</div>

	<!-- Footer -->
	{#if transcript.humanReviewed && transcript.reviewedAt}
		<div class="border-t border-gray-100 bg-gray-50 px-6 py-3 print:bg-white">
			<p class="text-xs text-gray-500">
				Geprüft am {new Date(transcript.reviewedAt).toLocaleDateString('de-DE', {
					day: '2-digit',
					month: '2-digit',
					year: 'numeric',
					hour: '2-digit',
					minute: '2-digit'
				})}
				{#if transcript.reviewNotes}
					<span class="mx-2">|</span>
					<span class="text-gray-600">{transcript.reviewNotes}</span>
				{/if}
			</p>
		</div>
	{/if}
</div>

<style>
	@media print {
		:global(body) {
			background: white !important;
		}
	}
</style>
