{{-- Toast Notifications Container --}}
<div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2"
     x-show="toasts.length > 0">
    <template x-for="toast in toasts" :key="toast.id">
        <div x-show="true"
             x-transition:enter="transition ease-out duration-300"
             x-transition:enter-start="opacity-0 translate-y-2"
             x-transition:enter-end="opacity-100 translate-y-0"
             x-transition:leave="transition ease-in duration-200"
             x-transition:leave-start="opacity-100"
             x-transition:leave-end="opacity-0"
             class="flex items-center gap-3 rounded-lg px-4 py-3 shadow-lg"
             :class="{
                 'bg-green-600 text-white': toast.type === 'success',
                 'bg-red-600 text-white': toast.type === 'error',
                 'bg-gray-800 text-white': toast.type === 'info'
             }">
            <template x-if="toast.type === 'success'">
                <x-lucide-check-circle class="h-5 w-5 flex-shrink-0" />
            </template>
            <template x-if="toast.type === 'error'">
                <x-lucide-x-circle class="h-5 w-5 flex-shrink-0" />
            </template>
            <template x-if="toast.type === 'info'">
                <x-lucide-info class="h-5 w-5 flex-shrink-0" />
            </template>
            <span x-text="toast.message" class="text-sm font-medium"></span>
            <button @click="dismiss(toast.id)" class="ml-2 hover:opacity-75">
                <x-lucide-x class="h-4 w-4" />
            </button>
        </div>
    </template>
</div>
