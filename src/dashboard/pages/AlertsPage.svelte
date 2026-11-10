<!-- Alerts and sounds: one row per event (design: alerts table). -->
<script lang="ts">
  import { Slider } from "glisse";
  import { react } from "../../lib/glass/bridge";
  import Icon, { type IconName } from "../../lib/ui/Icon.svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { settingsApi, type Alerts, type EventAlert, type Settings, type SoundName } from "../../lib/settings";
  import { act, change } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import Select from "../ui/Select.svelte";
  import Switch from "../ui/Switch.svelte";

  let { s }: { s: Settings } = $props();

  type EventKey = "done" | "error" | "permission" | "usage" | "chat" | "timer";
  const EVENTS: { id: EventKey; name: string; icon: IconName; tone: string; note?: string }[] = [
    { id: "done", name: "Task done", icon: "check", tone: "ok" },
    { id: "error", name: "Error", icon: "alert", tone: "err" },
    { id: "permission", name: "Permission request", icon: "agent", tone: "ink" },
    { id: "usage", name: "Usage warning", icon: "clock", tone: "ink" },
    { id: "chat", name: "Chat reply", icon: "send", tone: "ink" },
    { id: "timer", name: "Timer end", icon: "focus", tone: "ink", note: "With the focus timer" },
  ];
  const SOUNDS: { v: SoundName; label: string }[] = [
    { v: "chime", label: "Soft chime" },
    { v: "pop", label: "Pop" },
    { v: "bell", label: "Bell" },
    { v: "wood", label: "Wood block" },
    { v: "none", label: "None" },
  ];

  const al = $derived(s.alerts);
  const set = (id: EventKey, patch: Partial<EventAlert>) => {
    for (const [k, v] of Object.entries(patch)) change(`alerts.${id}.${k}`, v);
  };
  const off = $derived(!s.modules.alerts);
</script>

<Section title="Alerts">
  <SwitchRow
    label="Alerts"
    help="Turns every alert sound, flash and notification on or off."
    checked={s.modules.alerts}
    onchange={(v) => change("modules.alerts", v)}
  />
  <SwitchRow
    label="Flash the island"
    help="Pulses the island in the event's colour."
    checked={al.flash}
    disabled={off}
    onchange={(v) => change("alerts.flash", v)}
  />
</Section>

<Section title="Events" desc="Each event can play a sound and show a Windows notification. Set a sound to None to silence it.">
  <Row line={false}>
    {#snippet below()}
      <div class="table" class:d-dim={off} inert={off}>
        <div class="grid head" aria-hidden="true">
          <span>Event</span><span>Sound</span><span>Volume</span><span>Notification</span><span>Test</span>
        </div>
        {#each EVENTS as ev (ev.id)}
          {@const a = (al as Alerts)[ev.id] as EventAlert}
          {@const mute = a.sound === "none"}
          <div class="grid ev">
            <span class="name">
              <span class="ic" data-tone={ev.tone}><Icon name={ev.icon} size={16} /></span>
              <span class="nm">
                <span>{ev.name}</span>
                {#if ev.note}<span class="sub">{ev.note}</span>{/if}
              </span>
            </span>
            <Select
              label="Sound for {ev.name}"
              value={a.sound}
              options={SOUNDS}
              width="--size-alert-sound"
              onchange={(v) => set(ev.id, { sound: v as SoundName })}
            />
            <span class="vol">
              <span
                class="host"
                use:react={{
                  component: Slider,
                  props: {
                    value: a.volume,
                    min: 0,
                    max: 100,
                    step: 5,
                    disabled: mute,
                    "aria-label": `Volume for ${ev.name}`,
                    onValueChange: (v: number) => set(ev.id, { volume: v }),
                  },
                }}
              ></span>
              <span class="pct tabular" class:mute>{mute ? "Off" : `${a.volume}%`}</span>
            </span>
            <Switch checked={a.notify} label="Windows notification for {ev.name}" onchange={(v) => set(ev.id, { notify: v })} />
            <GButton
              icon="play"
              iconSize={14}
              ariaLabel="Preview the sound for {ev.name}"
              disabled={mute}
              onclick={() => act(() => settingsApi.previewSound(a.sound, a.volume), "")}
            />
          </div>
        {/each}
      </div>
    {/snippet}
  </Row>
</Section>

<style>
  .table {
    display: flex;
    flex-direction: column;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) var(--size-alert-sound) var(--size-alert-volume) var(--size-alert-notify) var(--size-control);
    gap: var(--sp-12);
    align-items: center;
  }
  .head {
    padding-bottom: var(--sp-8);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    color: var(--c-muted);
  }
  .ev {
    min-height: var(--size-alert-row);
    border-top: 1px solid var(--c-line);
  }
  .name {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    min-width: 0;
  }
  .nm {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .sub {
    font-size: var(--fs-label);
    color: var(--c-muted);
  }
  .ic {
    display: flex;
    flex: none;
  }
  .ic[data-tone="ok"] {
    color: var(--c-ok);
  }
  .ic[data-tone="err"] {
    color: var(--c-err);
  }
  .vol {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    --slider-w: var(--size-alert-slider);
  }
  .host {
    display: contents;
  }
  /* the design's tighter On / Off word in the narrow Notification column */
  .ev :global(.sw) {
    gap: var(--sp-4);
  }
  .pct {
    width: var(--size-alert-pct);
    font-size: var(--fs-label);
    color: var(--c-muted);
  }
  .pct.mute {
    color: var(--c-dis);
  }
</style>
