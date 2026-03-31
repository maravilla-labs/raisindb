import './bootstrap';
import Alpine from 'alpinejs';

// Initialize Alpine.js
window.Alpine = Alpine;

// Register Alpine.js components
Alpine.data('tagPicker', (config) => ({
    selectedTags: config.initialTags || [],
    availableTags: config.availableTags || [],
    searchQuery: '',
    isOpen: false,

    get flatTags() {
        const result = [];
        const addTag = (tag, depth = 0) => {
            result.push({ ...tag, depth });
            (tag.children || []).forEach(child => addTag(child, depth + 1));
        };
        this.availableTags.forEach(tag => addTag(tag));
        return result;
    },

    get filteredTags() {
        const selectedPaths = new Set(this.selectedTags.map(t => t['raisin:path']));
        let filtered = this.flatTags.filter(tag => !selectedPaths.has(tag.path));

        if (this.searchQuery.trim()) {
            const query = this.searchQuery.toLowerCase();
            filtered = filtered.filter(tag =>
                tag.name.toLowerCase().includes(query) ||
                (tag.properties?.label || '').toLowerCase().includes(query) ||
                tag.path.toLowerCase().includes(query)
            );
        }

        return filtered.slice(0, 10);
    },

    addTag(tag) {
        this.selectedTags.push({
            'raisin:ref': tag.id,
            'raisin:workspace': 'social',
            'raisin:path': tag.path
        });
        this.searchQuery = '';
        this.isOpen = false;
        this.updateHiddenInput();
    },

    removeTag(index) {
        this.selectedTags.splice(index, 1);
        this.updateHiddenInput();
    },

    getTagData(path) {
        return this.flatTags.find(t => t.path === path);
    },

    updateHiddenInput() {
        if (this.$refs.tagsInput) {
            this.$refs.tagsInput.value = JSON.stringify(this.selectedTags);
        }
    },

    handleFocus() {
        this.isOpen = true;
    },

    handleBlur() {
        setTimeout(() => { this.isOpen = false; }, 150);
    }
}));

Alpine.data('connectionPicker', (config) => ({
    connections: config.initialConnections || [],
    availableArticles: config.availableArticles || [],
    currentPath: config.currentPath || '',
    modalOpen: false,
    editingIndex: null,

    // Modal form state
    searchQuery: '',
    selectedArticle: null,
    relationType: 'similar-to',
    weight: 75,
    editorialNote: '',
    showDropdown: false,

    relationTypes: [
        { value: 'continues', label: 'Continues', description: 'This is a follow-up or sequel', color: '#3B82F6' },
        { value: 'updates', label: 'Updates', description: 'This is a newer version', color: '#8B5CF6' },
        { value: 'corrects', label: 'Corrects', description: 'This fixes or corrects information', color: '#F59E0B' },
        { value: 'contradicts', label: 'Contradicts', description: 'This presents an opposing view', color: '#EF4444' },
        { value: 'provides-evidence-for', label: 'Provides Evidence', description: 'Contains supporting data', color: '#22C55E' },
        { value: 'similar-to', label: 'Similar To', description: 'Covers related content', color: '#6B7280' },
        { value: 'see-also', label: 'See Also', description: 'Further reading', color: '#6B7280' },
    ],

    get filteredArticles() {
        const connectedPaths = new Set(this.connections.map(c => c.targetPath));
        let articles = this.availableArticles.filter(a =>
            a.path !== this.currentPath &&
            (this.editingIndex !== null || !connectedPaths.has(a.path))
        );

        if (this.searchQuery.trim()) {
            const query = this.searchQuery.toLowerCase();
            articles = articles.filter(a =>
                (a.properties?.title || '').toLowerCase().includes(query) ||
                a.path.toLowerCase().includes(query)
            );
        }

        return articles.slice(0, 10);
    },

    openAddModal() {
        this.editingIndex = null;
        this.resetModalForm();
        this.modalOpen = true;
    },

    openEditModal(index) {
        this.editingIndex = index;
        const conn = this.connections[index];
        this.selectedArticle = this.availableArticles.find(a => a.path === conn.targetPath);
        this.relationType = conn.relationType;
        this.weight = conn.weight;
        this.editorialNote = conn.editorialNote || '';
        this.modalOpen = true;
    },

    resetModalForm() {
        this.searchQuery = '';
        this.selectedArticle = null;
        this.relationType = 'similar-to';
        this.weight = 75;
        this.editorialNote = '';
        this.showDropdown = false;
    },

    selectArticle(article) {
        this.selectedArticle = article;
        this.searchQuery = '';
        this.showDropdown = false;
    },

    saveConnection() {
        if (!this.selectedArticle) return;

        const connection = {
            targetPath: this.selectedArticle.path,
            targetId: this.selectedArticle.id,
            targetTitle: this.selectedArticle.properties?.title || this.selectedArticle.name,
            relationType: this.relationType,
            weight: this.weight,
            editorialNote: this.editorialNote.trim() || undefined
        };

        if (this.editingIndex !== null) {
            this.connections[this.editingIndex] = connection;
        } else {
            this.connections.push(connection);
        }

        this.modalOpen = false;
        this.updateHiddenInput();
    },

    removeConnection(index) {
        this.connections.splice(index, 1);
        this.updateHiddenInput();
    },

    updateHiddenInput() {
        if (this.$refs.connectionsInput) {
            this.$refs.connectionsInput.value = JSON.stringify(this.connections);
        }
    },

    getRelationType(value) {
        return this.relationTypes.find(r => r.value === value);
    }
}));

Alpine.data('markdownPreview', () => ({
    showPreview: false,

    togglePreview() {
        this.showPreview = !this.showPreview;
    }
}));

Alpine.data('toastManager', () => ({
    toasts: [],

    show(type, message, duration = 5000) {
        const id = Date.now().toString();
        this.toasts.push({ id, type, message });

        setTimeout(() => {
            this.dismiss(id);
        }, duration);

        return id;
    },

    dismiss(id) {
        this.toasts = this.toasts.filter(t => t.id !== id);
    },

    success(message) {
        return this.show('success', message);
    },

    error(message) {
        return this.show('error', message);
    }
}));

Alpine.data('tagTreeBrowser', (config) => ({
    tags: config.tags || [],
    expandedPaths: new Set(),
    editingTag: null,
    showCreateModal: false,
    createParentPath: null,

    toggleExpand(path) {
        if (this.expandedPaths.has(path)) {
            this.expandedPaths.delete(path);
        } else {
            this.expandedPaths.add(path);
        }
    },

    isExpanded(path) {
        return this.expandedPaths.has(path);
    },

    openCreateModal(parentPath = null) {
        this.createParentPath = parentPath;
        this.showCreateModal = true;
    },

    openEditModal(tag) {
        this.editingTag = { ...tag };
    },

    closeModals() {
        this.showCreateModal = false;
        this.editingTag = null;
        this.createParentPath = null;
    }
}));

Alpine.data('slugGenerator', () => ({
    slugEdited: false,

    generateSlug(title) {
        if (this.slugEdited) return;

        const slug = title
            .toLowerCase()
            .trim()
            .replace(/[^\w\s-]/g, '')
            .replace(/[\s_-]+/g, '-')
            .replace(/^-+|-+$/g, '');

        if (this.$refs.slugInput) {
            this.$refs.slugInput.value = slug;
        }
    },

    markSlugEdited() {
        this.slugEdited = true;
    }
}));

Alpine.start();
