// SPDX-License-Identifier: BSL-1.1

import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { AuthProvider, useAuth } from './contexts/AuthContext'
import ProtectedRoute from './components/ProtectedRoute'
import Login from './pages/Login'
import ChangePassword from './pages/ChangePassword'
import AdminUsers from './pages/AdminUsers'
import IdentityUsers from './pages/IdentityUsers'
import Layout from './components/Layout'
import RepositoryLayout from './components/RepositoryLayout'
import RepositoryList from './pages/RepositoryList'
import Workspaces from './pages/Workspaces'
import WorkspaceDetail from './pages/WorkspaceDetail'
import ContentExplorer from './pages/ContentExplorer'
import WorkspaceSelector from './pages/WorkspaceSelector'
import BranchManagement from './pages/BranchManagement'
import NodeTypes from './pages/NodeTypes'
import NodeTypeEditor from './pages/NodeTypeEditor'
import Mixins from './pages/Mixins'
import MixinEditor from './pages/MixinEditor'
import Archetypes from './pages/Archetypes'
import ArchetypeEditor from './pages/ArchetypeEditor'
import ElementTypes from './pages/ElementTypes'
import ElementTypeEditor from './pages/ElementTypeEditor'
import Users from './pages/Users'
import UserEditor from './pages/UserEditor'
import Roles from './pages/Roles'
import RoleEditor from './pages/RoleEditor'
import Groups from './pages/Groups'
import GroupEditor from './pages/GroupEditor'
import EntityCircles from './pages/EntityCircles'
import EntityCircleEditor from './pages/EntityCircleEditor'
import RelationTypesManager from './pages/RelationTypesManager'
import Models from './pages/Models'
import RepositorySettings from './pages/RepositorySettings'
import RepositoryManagement from './pages/RepositoryManagement'
import TenantManagement from './pages/TenantManagement'
import UserProfile from './pages/UserProfile'
import TenantAiSettings from './pages/TenantAiSettings'
import TenantAuthSettings from './pages/TenantAuthSettings'
import DatabaseManagementShared from './components/DatabaseManagementShared'
import RocksDBManagement from './pages/RocksDBManagement'
import JobsManagement from './pages/management/JobsManagement'
import ExecutionLogs from './pages/management/ExecutionLogs'
import FlowExecutionMonitor from './pages/management/FlowExecutionMonitor'
import RepositoryExecutionLogs from './pages/RepositoryExecutionLogs'
import RepositoryFlows from './pages/RepositoryFlows'
import RepositoryInbox from './pages/RepositoryInbox'
import { SqlQuery } from './pages/SqlQuery'
import FunctionsIDE from './pages/functions/FunctionsIDE'
import SystemUpdatesPage from './pages/SystemUpdatesPage'
import PackagesRouter from './pages/packages'
import AgentsList from './pages/agents/AgentsList'
import AgentEditor from './pages/agents/AgentEditor'
import AgentDetail from './pages/agents/AgentDetail'
import ConversationTrace from './pages/agents/ConversationTrace'
import AccessControlSettings from './pages/AccessControlSettings'
import Integrations from './pages/Integrations'
import McpConnections from './pages/McpConnections'
import Mounts from './pages/Mounts'
import Secrets from './pages/Secrets'
import MountDetail from './pages/MountDetail'

/**
 * Gate the tenant-wide /management hub to dev / single-operator mode.
 *
 * The hub exposes server-config style operations (RocksDB compaction,
 * dependency listing, etc.) that don't make sense to expose to a customer
 * inside their tenant view. In multi-tenant deployments those ops live
 * under /management/admin/* and require a superadmin token.
 *
 * Convention: tenantId === 'default' means single-operator local dev. Any
 * other resolved tenant means we're inside a customer's view and the hub
 * is hidden — direct navigation redirects to the repository list.
 */
function DevModeOnly({ children }: { children: React.ReactNode }) {
  const { tenantId, isLoading } = useAuth()
  if (isLoading) return null
  if (tenantId !== 'default') {
    return <Navigate to="/" replace />
  }
  return <>{children}</>
}

function App() {
  return (
    <BrowserRouter basename="/admin">
      <AuthProvider>
        <Routes>
          {/* Public routes */}
          <Route path="/login" element={<Login />} />

          {/* Protected routes */}
          <Route path="/change-password" element={<ProtectedRoute><ChangePassword /></ProtectedRoute>} />

          {/* Repository-scoped routes */}
          <Route path="/:repo" element={<ProtectedRoute><RepositoryLayout /></ProtectedRoute>}>
          <Route index element={<Navigate to="content" replace />} />
          <Route path="content" element={<WorkspaceSelector />} />
          <Route path="content/:branch/:workspace/*" element={<ContentExplorer />} />
          <Route path="workspaces" element={<Workspaces />} />
          <Route path="workspaces/:workspace" element={<WorkspaceDetail />} />
          <Route path="models" element={<Models />} />
          <Route path=":branch/models" element={<Models />} />
          {/* NodeTypes routes - support both with and without branch */}
          <Route path="nodetypes" element={<NodeTypes />} />
          <Route path=":branch/nodetypes" element={<NodeTypes />} />
          <Route path="nodetypes/new" element={<NodeTypeEditor />} />
          <Route path=":branch/nodetypes/new" element={<NodeTypeEditor />} />
          <Route path="nodetypes/:name" element={<NodeTypeEditor />} />
          <Route path=":branch/nodetypes/:name" element={<NodeTypeEditor />} />

          {/* Mixin routes (NodeTypes with is_mixin=true) */}
          <Route path="mixins" element={<Mixins />} />
          <Route path=":branch/mixins" element={<Mixins />} />
          <Route path="mixins/new" element={<MixinEditor />} />
          <Route path=":branch/mixins/new" element={<MixinEditor />} />
          <Route path="mixins/:name" element={<MixinEditor />} />
          <Route path=":branch/mixins/:name" element={<MixinEditor />} />
          {/* Archetype routes */}
          <Route path="archetypes" element={<Archetypes />} />
          <Route path=":branch/archetypes" element={<Archetypes />} />
          <Route path="archetypes/new" element={<ArchetypeEditor />} />
          <Route path=":branch/archetypes/new" element={<ArchetypeEditor />} />
          <Route path="archetypes/:name" element={<ArchetypeEditor />} />
          <Route path=":branch/archetypes/:name" element={<ArchetypeEditor />} />
          {/* ElementType routes */}
          <Route path="elementtypes" element={<ElementTypes />} />
          <Route path=":branch/elementtypes" element={<ElementTypes />} />
          <Route path="elementtypes/new" element={<ElementTypeEditor />} />
          <Route path=":branch/elementtypes/new" element={<ElementTypeEditor />} />
          <Route path="elementtypes/:name" element={<ElementTypeEditor />} />
          <Route path=":branch/elementtypes/:name" element={<ElementTypeEditor />} />
          {/* Users routes */}
          <Route path="users/new" element={<UserEditor />} />
          <Route path=":branch/users/new" element={<UserEditor />} />
          <Route path="users/*" element={<Users />} />
          <Route path=":branch/users/*" element={<Users />} />
          <Route path="users" element={<Users />} />
          <Route path=":branch/users" element={<Users />} />
          {/* Roles routes */}
          <Route path="roles/new" element={<RoleEditor />} />
          <Route path=":branch/roles/new" element={<RoleEditor />} />
          <Route path="roles/*" element={<Roles />} />
          <Route path=":branch/roles/*" element={<Roles />} />
          <Route path="roles" element={<Roles />} />
          <Route path=":branch/roles" element={<Roles />} />
          {/* Groups routes */}
          <Route path="groups/new" element={<GroupEditor />} />
          <Route path=":branch/groups/new" element={<GroupEditor />} />
          <Route path="groups/*" element={<Groups />} />
          <Route path=":branch/groups/*" element={<Groups />} />
          <Route path="groups" element={<Groups />} />
          <Route path=":branch/groups" element={<Groups />} />
          {/* Entity Circles routes */}
          <Route path="circles/new" element={<EntityCircleEditor />} />
          <Route path=":branch/circles/new" element={<EntityCircleEditor />} />
          <Route path="circles/*" element={<EntityCircles />} />
          <Route path=":branch/circles/*" element={<EntityCircles />} />
          <Route path="circles" element={<EntityCircles />} />
          <Route path=":branch/circles" element={<EntityCircles />} />
          {/* Relation Types routes */}
          <Route path="relation-types" element={<RelationTypesManager />} />
          <Route path=":branch/relation-types" element={<RelationTypesManager />} />
          {/* Access Control Settings */}
          <Route path="access-control/settings" element={<AccessControlSettings />} />
          <Route path=":branch/access-control/settings" element={<AccessControlSettings />} />
          <Route path="branches" element={<BranchManagement />} />
          {/* Functions IDE - matches ContentExplorer pattern */}
          <Route path="functions" element={<FunctionsIDE />} />
          <Route path="functions/:branch/*" element={<FunctionsIDE />} />
          {/* Agents routes */}
          <Route path="agents" element={<AgentsList />} />
          <Route path=":branch/agents" element={<AgentsList />} />
          <Route path="agents/new" element={<AgentEditor />} />
          <Route path=":branch/agents/new" element={<AgentEditor />} />
          <Route path="agents/:agentId" element={<AgentDetail />} />
          <Route path=":branch/agents/:agentId" element={<AgentDetail />} />
          <Route path="agents/:agentId/edit" element={<AgentEditor />} />
          <Route path=":branch/agents/:agentId/edit" element={<AgentEditor />} />
          <Route path="agents/:agentId/conversations/:conversationPath" element={<ConversationTrace />} />
          <Route path=":branch/agents/:agentId/conversations/:conversationPath" element={<ConversationTrace />} />
          {/* Packages routes */}
          <Route path="packages/*" element={<PackagesRouter />} />
          <Route path=":branch/packages/*" element={<PackagesRouter />} />
          {/* Outbound integrations + virtual mounts (raisin:system workspace) */}
          <Route path="integrations" element={<Integrations />} />
          <Route path="mcp-connections" element={<McpConnections />} />
          <Route path="mounts" element={<Mounts />} />
          {/* Sidebar highlighting is prefix-based (`isActive('/mounts')` in
              RepositoryLayout), so the nested detail route keeps Mounts lit. */}
          <Route path="mounts/:mountName" element={<MountDetail />} />
          {/* Secrets are scoped to a BRANCH, so both forms are registered: the
              branchless one falls back to `main`, and the branch switcher
              rewrites to the `:branch` form (see RepositoryLayout's routeTypes). */}
          <Route path="secrets" element={<Secrets />} />
          <Route path=":branch/secrets" element={<Secrets />} />
          <Route path="query" element={<SqlQuery />} />
          <Route path="logs" element={<RepositoryExecutionLogs />} />
          <Route path="flows" element={<RepositoryFlows />} />
          <Route path="inbox" element={<RepositoryInbox />} />
          <Route path="settings/*" element={<RepositorySettings />} />
          <Route path="system-updates" element={<SystemUpdatesPage />} />
          <Route path="management/*" element={<RepositoryManagement />} />
        </Route>

          {/* Layout-wrapped protected routes — sidebar + header chrome.
              Both the landing page (`/` → RepositoryList) and the
              `/management/*` hub render inside the same Layout so the
              sidebar is reachable from `/admin` directly after login,
              not only from /management/*.

              Per-tenant /management routes (admin-users, identity-users,
              auth, ai, flows, database, logs, profile, jobs) are visible
              for any authenticated tenant — they call tenant-scoped
              backend endpoints via the v0.1.19 ScopedTenant fix.

              Operator-only routes (the index Dashboard, rocksdb) are
              wrapped in <DevModeOnly> so they only render for
              tenantId === 'default'. Other tenants are redirected to the
              repository list. Cross-tenant ops live on the superadmin
              routes (/management/admin/*) instead. */}
          <Route element={<ProtectedRoute><Layout /></ProtectedRoute>}>
            <Route path="/" element={<RepositoryList />} />
            <Route path="/management">
              <Route index element={<DevModeOnly><TenantManagement /></DevModeOnly>} />
              <Route path="rocksdb" element={<DevModeOnly><RocksDBManagement /></DevModeOnly>} />
              <Route path="jobs" element={<JobsManagement />} />

              <Route
                path="database"
                element={
                  <DatabaseManagementShared
                    showBranchSelector={true}
                    context="tenant"
                  />
                }
              />
              <Route path="ai" element={<TenantAiSettings />} />
              <Route path="auth" element={<TenantAuthSettings />} />
              <Route path="logs" element={<ExecutionLogs />} />
              <Route path="flows" element={<FlowExecutionMonitor />} />
              <Route path="admin-users" element={<AdminUsers />} />
              <Route path="identity-users" element={<IdentityUsers />} />
              <Route path="profile" element={<UserProfile />} />
            </Route>
          </Route>
        </Routes>
      </AuthProvider>
    </BrowserRouter>
  )
}

export default App
