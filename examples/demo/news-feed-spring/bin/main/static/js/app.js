// News Feed Spring Demo - Client-side JavaScript

document.addEventListener('DOMContentLoaded', function() {
    // Auto-generate slug from title
    const titleInput = document.getElementById('title');
    const slugInput = document.getElementById('slug');

    if (titleInput && slugInput) {
        titleInput.addEventListener('input', function() {
            // Only auto-generate if slug is empty or was previously auto-generated
            if (!slugInput.dataset.manual) {
                slugInput.value = generateSlug(titleInput.value);
            }
        });

        slugInput.addEventListener('input', function() {
            slugInput.dataset.manual = 'true';
        });
    }

    // Confirm delete actions
    document.querySelectorAll('[data-confirm]').forEach(function(element) {
        element.addEventListener('click', function(e) {
            if (!confirm(element.dataset.confirm)) {
                e.preventDefault();
            }
        });
    });
});

/**
 * Generate a URL-friendly slug from text
 */
function generateSlug(text) {
    return text
        .toLowerCase()
        .trim()
        .replace(/[^\w\s-]/g, '')
        .replace(/[\s_-]+/g, '-')
        .replace(/^-+|-+$/g, '');
}

/**
 * Delete article via API
 */
async function deleteArticle(category, slug) {
    if (!confirm('Are you sure you want to delete this article? This action cannot be undone.')) {
        return;
    }

    try {
        const response = await fetch(`/api/articles/${category}/${slug}`, {
            method: 'DELETE',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (response.ok) {
            window.location.href = '/';
        } else {
            const data = await response.json();
            alert('Failed to delete article: ' + (data.error || 'Unknown error'));
        }
    } catch (error) {
        alert('Failed to delete article: ' + error.message);
    }
}
