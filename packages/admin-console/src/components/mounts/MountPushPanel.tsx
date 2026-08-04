// SPDX-License-Identifier: BSL-1.1

import { AlertTriangle, CheckCircle2, Inbox, Radio, XCircle } from 'lucide-react'
import type { MountState } from '../../api/integrations'
import { pushDeliveryHealth } from '../../utils/mountStatus'
import CopyableUrlField from '../integrations/CopyableUrlField'
import {
  dateFromIso,
  formatAbsolute,
  formatAbsoluteSeconds,
  formatRelative,
  formatRelativeSeconds,
} from '../../utils/time'

interface MountPushPanelProps {
  state?: MountState
}

/**
 * Webhook health, split into the two things that are constantly conflated:
 * whether a SUBSCRIPTION exists, and whether DELIVERIES are getting through.
 *
 * This split is the point of the panel. The production stall that motivated it
 * showed `push_status: "active"` with an unexpired subscription the entire
 * time — every subscription-lifecycle field said healthy — while every inbound
 * notification was being rejected. Nothing in the old UI could tell those apart,
 * so the mount read as fine while it imported nothing for weeks.
 *
 * Delivery therefore leads, and a rejecting mount is called out as the headline,
 * not left to be inferred from two adjacent counters.
 */
export default function MountPushPanel({ state }: MountPushPanelProps) {
  const status = (state?.push_status || '').toLowerCase()
  const configured = !!status || !!state?.push_notification_url
  const health = pushDeliveryHealth(state)

  if (!configured) {
    return (
      <p className="text-sm text-zinc-500">
        This mount is poll-only — it has no push subscription, so it syncs on its schedule rather
        than on notification.
      </p>
    )
  }

  const okCount = state?.push_deliveries_ok ?? 0
  const rejectedCount = state?.push_deliveries_rejected ?? 0
  const expiry = dateFromIso(state?.push_expires_at)
  const expired = expiry ? expiry.getTime() < Date.now() : false

  const HEALTH: Record<
    string,
    { title: string; body: string; cls: string; Icon: typeof CheckCircle2 }
  > = {
    healthy: {
      title: 'Deliveries are arriving',
      body: 'The most recent notification was accepted and processed.',
      cls: 'bg-green-500/10 border-green-500/20 text-green-400',
      Icon: CheckCircle2,
    },
    rejecting: {
      title: 'Notifications are being rejected',
      body:
        'The provider is delivering and the subscription looks active, but this server is ' +
        'refusing the notifications — so nothing is syncing on push. This is the state that ' +
        'looks healthy from every other angle.',
      cls: 'bg-red-500/10 border-red-500/20 text-red-400',
      Icon: XCircle,
    },
    silent: {
      title: 'No recent deliveries',
      body:
        'Notifications were accepted before but none has arrived lately, while the subscription ' +
        'is still nominally active. The provider may have stopped sending.',
      cls: 'bg-yellow-500/10 border-yellow-500/20 text-yellow-400',
      Icon: AlertTriangle,
    },
    none: {
      title: 'No delivery yet',
      body: 'A subscription exists but no notification has ever reached this server.',
      cls: 'bg-zinc-500/10 border-white/10 text-zinc-400',
      Icon: Inbox,
    },
  }
  const h = HEALTH[health]

  return (
    <div className="space-y-4">
      {/* Headline: delivery, not subscription. */}
      <div className={`rounded-lg border p-3 flex items-start gap-2.5 ${h.cls}`}>
        <h.Icon className="w-4 h-4 mt-0.5 shrink-0" />
        <div className="min-w-0">
          <p className="text-sm font-medium">{h.title}</p>
          <p className="text-xs opacity-80 mt-0.5 leading-relaxed">{h.body}</p>
          {health === 'rejecting' && state?.push_last_rejected_reason && (
            <p className="text-xs font-mono mt-1.5 bg-black/30 rounded px-2 py-1 break-words">
              {state.push_last_rejected_reason}
            </p>
          )}
        </div>
      </div>

      {/* Delivery counters */}
      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-lg border border-white/10 bg-white/[0.02] px-3 py-2.5">
          <div className="text-[11px] uppercase tracking-wide text-zinc-500 mb-1">Accepted</div>
          <div
            className={`text-xl font-semibold tabular-nums ${okCount ? 'text-green-400' : 'text-zinc-500'}`}
          >
            {okCount.toLocaleString()}
          </div>
          <div className="text-[11px] text-zinc-500 mt-0.5" title={formatAbsoluteSeconds(state?.push_last_delivery_at)}>
            last {formatRelativeSeconds(state?.push_last_delivery_at)}
          </div>
        </div>
        <div className="rounded-lg border border-white/10 bg-white/[0.02] px-3 py-2.5">
          <div className="text-[11px] uppercase tracking-wide text-zinc-500 mb-1">Rejected</div>
          <div
            className={`text-xl font-semibold tabular-nums ${rejectedCount ? 'text-red-400' : 'text-zinc-500'}`}
          >
            {rejectedCount.toLocaleString()}
          </div>
          <div className="text-[11px] text-zinc-500 mt-0.5" title={formatAbsoluteSeconds(state?.push_last_rejected_at)}>
            last {formatRelativeSeconds(state?.push_last_rejected_at)}
          </div>
        </div>
      </div>

      {/* Subscription lifecycle — secondary, explicitly labelled as a different thing. */}
      <div className="pt-1">
        <div className="flex items-center gap-1.5 mb-2">
          <Radio className="w-3.5 h-3.5 text-zinc-500" />
          <h4 className="text-xs uppercase tracking-wide text-zinc-500">Subscription</h4>
        </div>
        <dl className="space-y-1.5 text-sm">
          <Row label="Status">
            <span
              className={
                status === 'active' && !expired
                  ? 'text-green-400'
                  : status === 'failed' || status === 'error' || expired
                    ? 'text-red-400'
                    : 'text-zinc-300'
              }
            >
              {status || 'unknown'}
              {expired && ' (expired)'}
            </span>
          </Row>
          {state?.push_subscription_id && (
            <Row label="Provider id">
              <span className="font-mono text-xs text-zinc-400 break-all">
                {state.push_subscription_id}
              </span>
            </Row>
          )}
          {expiry && (
            <Row label="Renews">
              <span
                className={expired ? 'text-red-400' : 'text-zinc-300'}
                title={formatAbsolute(expiry)}
              >
                {formatRelative(expiry)}
              </span>
            </Row>
          )}
          {state?.push_last_error && (
            <Row label="Last error">
              <span className="text-red-400 text-xs break-words">{state.push_last_error}</span>
            </Row>
          )}
        </dl>
      </div>

      {state?.push_notification_url && (
        <CopyableUrlField
          label="Notification URL"
          value={state.push_notification_url}
          helper="The provider must POST notifications here. Rejections often mean this URL, or the token in it, no longer matches what the provider was registered with."
        />
      )}
    </div>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-3">
      <dt className="text-zinc-500 text-xs w-24 shrink-0">{label}</dt>
      <dd className="min-w-0 flex-1">{children}</dd>
    </div>
  )
}
