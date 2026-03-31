package com.raisindb.newsfeed.util;

import java.util.Map;

/**
 * Relation types for article connections with metadata.
 */
public final class RelationTypes {

    public static final String CONTINUES = "continues";
    public static final String UPDATES = "updates";
    public static final String CORRECTS = "corrects";
    public static final String CONTRADICTS = "contradicts";
    public static final String PROVIDES_EVIDENCE_FOR = "provides-evidence-for";
    public static final String SIMILAR_TO = "similar-to";
    public static final String SEE_ALSO = "see-also";

    private RelationTypes() {
    }

    /**
     * Metadata for each relation type.
     */
    public static class RelationMeta {
        private final String label;
        private final String description;
        private final String color;
        private final String icon;

        public RelationMeta(String label, String description, String color, String icon) {
            this.label = label;
            this.description = description;
            this.color = color;
            this.icon = icon;
        }

        public String getLabel() {
            return label;
        }

        public String getDescription() {
            return description;
        }

        public String getColor() {
            return color;
        }

        public String getIcon() {
            return icon;
        }
    }

    public static final Map<String, RelationMeta> RELATION_TYPE_META = Map.of(
            CONTINUES, new RelationMeta(
                    "Continues",
                    "This is a follow-up or sequel to the target article",
                    "#3B82F6",
                    "arrow-right"
            ),
            UPDATES, new RelationMeta(
                    "Updates",
                    "This is a newer version or update of the target story",
                    "#8B5CF6",
                    "refresh-cw"
            ),
            CORRECTS, new RelationMeta(
                    "Corrects",
                    "This article fixes or corrects information in the target",
                    "#F59E0B",
                    "pencil"
            ),
            CONTRADICTS, new RelationMeta(
                    "Contradicts",
                    "This article presents an opposing or conflicting view",
                    "#EF4444",
                    "x-circle"
            ),
            PROVIDES_EVIDENCE_FOR, new RelationMeta(
                    "Provides Evidence",
                    "This article contains supporting data or sources",
                    "#22C55E",
                    "file-check"
            ),
            SIMILAR_TO, new RelationMeta(
                    "Similar To",
                    "This article covers related or similar content",
                    "#6B7280",
                    "link"
            ),
            SEE_ALSO, new RelationMeta(
                    "See Also",
                    "Editorial recommendation for further reading",
                    "#6B7280",
                    "bookmark"
            )
    );

    /**
     * Get metadata for a relation type.
     */
    public static RelationMeta getMeta(String relationType) {
        return RELATION_TYPE_META.get(relationType);
    }

    /**
     * Get all relation types as a list.
     */
    public static String[] getAllTypes() {
        return new String[]{
                CONTINUES, UPDATES, CORRECTS, CONTRADICTS,
                PROVIDES_EVIDENCE_FOR, SIMILAR_TO, SEE_ALSO
        };
    }
}
