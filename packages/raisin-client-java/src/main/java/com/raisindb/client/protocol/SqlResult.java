package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.List;

/**
 * Result of a SQL query.
 */
public class SqlResult {

    @JsonProperty("columns")
    private List<String> columns;

    @JsonProperty("rows")
    private List<List<Object>> rows;

    @JsonProperty("row_count")
    private int rowCount;

    public SqlResult() {
    }

    public SqlResult(List<String> columns, List<List<Object>> rows, int rowCount) {
        this.columns = columns;
        this.rows = rows;
        this.rowCount = rowCount;
    }

    // Getters and setters
    public List<String> getColumns() { return columns; }
    public void setColumns(List<String> columns) { this.columns = columns; }

    public List<List<Object>> getRows() { return rows; }
    public void setRows(List<List<Object>> rows) { this.rows = rows; }

    public int getRowCount() { return rowCount; }
    public void setRowCount(int rowCount) { this.rowCount = rowCount; }
}
