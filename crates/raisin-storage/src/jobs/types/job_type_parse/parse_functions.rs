// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parsers for function execution, flow execution, and AI job types

use super::super::job_type::JobType;

pub(crate) fn parse_function_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("FunctionExecution(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::FunctionExecution {
                    function_path: p[0].to_string(),
                    trigger_name: None,
                    execution_id: p[1].to_string(),
                }));
            } else if p.len() >= 3 {
                let fp = p[..p.len() - 2].join("/");
                return Ok(Some(JobType::FunctionExecution {
                    function_path: fp,
                    trigger_name: Some(p[p.len() - 2].to_string()),
                    execution_id: p[p.len() - 1].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("TriggerEvaluation(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 3 {
                return Ok(Some(JobType::TriggerEvaluation {
                    event_type: p[0].to_string(),
                    node_id: p[1].to_string(),
                    node_type: p[2].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("ScheduledTriggerCheck(") {
        if let Some(c) = rest.strip_suffix(')') {
            if c == "*" {
                return Ok(Some(JobType::ScheduledTriggerCheck {
                    tenant_id: None,
                    repo_id: None,
                }));
            }
            let p: Vec<&str> = c.split('/').collect();
            match p.len() {
                1 => {
                    return Ok(Some(JobType::ScheduledTriggerCheck {
                        tenant_id: Some(p[0].to_string()),
                        repo_id: None,
                    }))
                }
                2 => {
                    let tid = if p[0] == "*" {
                        None
                    } else {
                        Some(p[0].to_string())
                    };
                    return Ok(Some(JobType::ScheduledTriggerCheck {
                        tenant_id: tid,
                        repo_id: Some(p[1].to_string()),
                    }));
                }
                _ => {}
            }
        }
    }
    Ok(None)
}

pub(crate) fn parse_flow_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("FlowExecution(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() >= 3 {
                if let Some(ss) = p[p.len() - 1].strip_prefix("step:") {
                    let si = ss
                        .parse::<usize>()
                        .map_err(|_| format!("Invalid step index: {}", ss))?;
                    let tp = p[..p.len() - 2].join("/");
                    return Ok(Some(JobType::FlowExecution {
                        flow_execution_id: p[p.len() - 2].to_string(),
                        trigger_path: tp,
                        flow: serde_json::Value::Null,
                        current_step_index: si,
                        step_results: serde_json::Value::Null,
                    }));
                }
            }
        }
    }
    if let Some(rest) = s.strip_prefix("FlowInstanceExecution(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::FlowInstanceExecution {
                    instance_id: p[0].to_string(),
                    execution_type: p[1].to_string(),
                    resume_reason: None,
                }));
            } else if p.len() == 3 {
                return Ok(Some(JobType::FlowInstanceExecution {
                    instance_id: p[0].to_string(),
                    execution_type: p[1].to_string(),
                    resume_reason: Some(p[2].to_string()),
                }));
            }
        }
    }
    Ok(None)
}

pub(crate) fn parse_ai_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("AICall(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() >= 4 {
                let iter = p[p.len() - 1]
                    .parse::<u32>()
                    .map_err(|_| format!("Invalid iteration: {}", p[p.len() - 1]))?;
                let ar = if p.len() > 4 {
                    p[2..p.len() - 1].join("/")
                } else {
                    p[2].to_string()
                };
                return Ok(Some(JobType::AICall {
                    instance_id: p[0].to_string(),
                    step_id: p[1].to_string(),
                    agent_ref: ar,
                    iteration: iter,
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("AIToolCallExecution(") {
        if let Some(c) = rest.strip_suffix(')') {
            if let Some((ws, path)) = c.split_once('/') {
                return Ok(Some(JobType::AIToolCallExecution {
                    tool_call_workspace: ws.to_string(),
                    tool_call_path: format!("/{}", path),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("AIToolResultAggregation(") {
        if let Some(c) = rest.strip_suffix(')') {
            if let Some((ws, path)) = c.split_once('/') {
                return Ok(Some(JobType::AIToolResultAggregation {
                    workspace: ws.to_string(),
                    single_result_path: format!("/{}", path),
                }));
            }
        }
    }
    Ok(None)
}
