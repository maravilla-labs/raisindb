package com.raisindb.newsfeed.util;

/**
 * Utility class for path conversions between database paths and URL paths.
 */
public final class PathUtils {

    public static final String BASE_PATH = "/superbigshit";
    public static final String ARTICLES_PATH = BASE_PATH + "/articles";
    public static final String TAGS_PATH = BASE_PATH + "/tags";

    private PathUtils() {
    }

    /**
     * Convert a database path to a URL path (remove base path).
     * e.g., /superbigshit/articles/tech/my-article -> /articles/tech/my-article
     */
    public static String pathToUrl(String dbPath) {
        if (dbPath == null || dbPath.isEmpty()) {
            return "/";
        }
        if (dbPath.startsWith(BASE_PATH)) {
            String result = dbPath.substring(BASE_PATH.length());
            return result.isEmpty() ? "/" : result;
        }
        return dbPath;
    }

    /**
     * Convert a URL path to a database path (add base path).
     * e.g., /articles/tech/my-article -> /superbigshit/articles/tech/my-article
     */
    public static String urlToPath(String urlPath) {
        if (urlPath == null || urlPath.isEmpty()) {
            return ARTICLES_PATH;
        }
        if (urlPath.startsWith("/articles")) {
            return BASE_PATH + urlPath;
        }
        return ARTICLES_PATH + urlPath;
    }

    /**
     * Build article path from category and slug.
     */
    public static String buildArticlePath(String category, String slug) {
        return ARTICLES_PATH + "/" + category + "/" + slug;
    }

    /**
     * Build tag path from parent path and tag name.
     */
    public static String buildTagPath(String parentPath, String tagName) {
        return parentPath + "/" + tagName;
    }

    /**
     * Extract category slug from article path.
     * e.g., /superbigshit/articles/tech/my-article -> tech
     */
    public static String getCategoryFromPath(String path) {
        if (path == null || path.isEmpty()) {
            return "";
        }
        String[] parts = path.split("/");
        for (int i = 0; i < parts.length; i++) {
            if ("articles".equals(parts[i]) && i + 1 < parts.length) {
                return parts[i + 1];
            }
        }
        return "";
    }

    /**
     * Extract the last segment of a path (slug or name).
     */
    public static String getSlugFromPath(String path) {
        if (path == null || path.isEmpty()) {
            return "";
        }
        String[] parts = path.split("/");
        return parts.length > 0 ? parts[parts.length - 1] : "";
    }

    /**
     * Get parent path.
     */
    public static String getParentPath(String path) {
        if (path == null || path.isEmpty()) {
            return "";
        }
        int lastSlash = path.lastIndexOf('/');
        return lastSlash > 0 ? path.substring(0, lastSlash) : "";
    }

    /**
     * Escape single quotes for SQL.
     */
    public static String escapeSql(String value) {
        if (value == null) {
            return "";
        }
        return value.replace("'", "''");
    }
}
