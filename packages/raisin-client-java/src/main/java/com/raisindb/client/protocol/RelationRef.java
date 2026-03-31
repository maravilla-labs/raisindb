package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * A reference to a related node in the graph database.
 *
 * Represents a directed relationship from a source node to a target node,
 * potentially across workspace boundaries.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public class RelationRef {

    @JsonProperty("target")
    private String target;

    @JsonProperty("workspace")
    private String workspace;

    @JsonProperty("target_node_type")
    private String targetNodeType;

    @JsonProperty("relation_type")
    private String relationType;

    @JsonProperty("weight")
    private Float weight;

    public RelationRef() {
    }

    public RelationRef(String target, String workspace, String targetNodeType,
                      String relationType, Float weight) {
        this.target = target;
        this.workspace = workspace;
        this.targetNodeType = targetNodeType;
        this.relationType = relationType;
        this.weight = weight;
    }

    // Getters and setters
    public String getTarget() { return target; }
    public void setTarget(String target) { this.target = target; }

    public String getWorkspace() { return workspace; }
    public void setWorkspace(String workspace) { this.workspace = workspace; }

    public String getTargetNodeType() { return targetNodeType; }
    public void setTargetNodeType(String targetNodeType) { this.targetNodeType = targetNodeType; }

    public String getRelationType() { return relationType; }
    public void setRelationType(String relationType) { this.relationType = relationType; }

    public Float getWeight() { return weight; }
    public void setWeight(Float weight) { this.weight = weight; }

    @Override
    public String toString() {
        return String.format("RelationRef{target='%s', relationType='%s', targetNodeType='%s'}",
                           target, relationType, targetNodeType);
    }
}
