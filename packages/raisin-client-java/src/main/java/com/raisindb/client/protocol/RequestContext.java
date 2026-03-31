package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * Context for a request (tenant, repository, workspace, branch).
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public class RequestContext {

    @JsonProperty("tenant_id")
    private String tenantId;

    @JsonProperty("repository")
    private String repository;

    @JsonProperty("branch")
    private String branch;

    @JsonProperty("workspace")
    private String workspace;

    /** Revision/commit ID for time-travel queries (optional) */
    @JsonProperty("revision")
    private String revision;

    public RequestContext() {
    }

    public RequestContext(String tenantId) {
        this.tenantId = tenantId;
    }

    public RequestContext(String tenantId, String repository) {
        this.tenantId = tenantId;
        this.repository = repository;
    }

    public RequestContext(String tenantId, String repository, String workspace, String branch) {
        this.tenantId = tenantId;
        this.repository = repository;
        this.workspace = workspace;
        this.branch = branch;
    }

    // Builder pattern
    public static Builder builder() {
        return new Builder();
    }

    public static class Builder {
        private String tenantId;
        private String repository;
        private String branch;
        private String workspace;
        private String revision;

        public Builder tenantId(String tenantId) {
            this.tenantId = tenantId;
            return this;
        }

        public Builder repository(String repository) {
            this.repository = repository;
            return this;
        }

        public Builder branch(String branch) {
            this.branch = branch;
            return this;
        }

        public Builder workspace(String workspace) {
            this.workspace = workspace;
            return this;
        }

        public Builder revision(String revision) {
            this.revision = revision;
            return this;
        }

        public RequestContext build() {
            RequestContext context = new RequestContext();
            context.tenantId = this.tenantId;
            context.repository = this.repository;
            context.branch = this.branch;
            context.workspace = this.workspace;
            context.revision = this.revision;
            return context;
        }
    }

    // Getters and setters
    public String getTenantId() { return tenantId; }
    public void setTenantId(String tenantId) { this.tenantId = tenantId; }

    public String getRepository() { return repository; }
    public void setRepository(String repository) { this.repository = repository; }

    public String getBranch() { return branch; }
    public void setBranch(String branch) { this.branch = branch; }

    public String getWorkspace() { return workspace; }
    public void setWorkspace(String workspace) { this.workspace = workspace; }

    public String getRevision() { return revision; }
    public void setRevision(String revision) { this.revision = revision; }
}
