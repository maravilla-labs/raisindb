package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * Metadata for streaming and pagination.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public class ResponseMetadata {

    @JsonProperty("chunk")
    private Integer chunk;

    @JsonProperty("total_chunks")
    private Integer totalChunks;

    @JsonProperty("total")
    private Long total;

    @JsonProperty("has_more")
    private Boolean hasMore;

    @JsonProperty("credits_consumed")
    private Integer creditsConsumed;

    public ResponseMetadata() {
    }

    // Getters and setters
    public Integer getChunk() { return chunk; }
    public void setChunk(Integer chunk) { this.chunk = chunk; }

    public Integer getTotalChunks() { return totalChunks; }
    public void setTotalChunks(Integer totalChunks) { this.totalChunks = totalChunks; }

    public Long getTotal() { return total; }
    public void setTotal(Long total) { this.total = total; }

    public Boolean getHasMore() { return hasMore; }
    public void setHasMore(Boolean hasMore) { this.hasMore = hasMore; }

    public Integer getCreditsConsumed() { return creditsConsumed; }
    public void setCreditsConsumed(Integer creditsConsumed) { this.creditsConsumed = creditsConsumed; }
}
