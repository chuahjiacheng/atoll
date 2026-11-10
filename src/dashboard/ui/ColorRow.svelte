<!--
  Accent / status colour (design: colour row). The trigger shows the colour
  in use; it opens the presets and a custom hex field. A custom colour is
  only saved when it has at least 4.5:1 contrast on the island.
-->
<script lang="ts" module>
  export const PRESETS = {
    accent: [
      ["peri", "Periwinkle"],
      ["teal", "Teal"],
      ["orchid", "Orchid"],
      ["coral", "Coral"],
    ],
    busy: [
      ["amber", "Amber"],
      ["orange", "Orange"],
    ],
    done: [
      ["green", "Green"],
      ["teal", "Teal"],
    ],
    error: [
      ["red", "Red"],
      ["pink", "Pink"],
    ],
  } as const;
</script>

<script lang="ts">
  import { tick } from "svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { contrast, hullHex, isHex, luminance, MIN_CONTRAST, token, type ColorKey, type Theme } from "../../lib/theme";
  import type { ColorChoice } from "../../lib/settings";
  import { dur, ease, gsap, reduced } from "../../lib/motion";
  import Row from "./Row.svelte";

  let {
    key,
    label,
    help,
    choice,
    theme,
    open,
    ontoggle,
    onchange,
  }: {
    key: ColorKey;
    label: string;
    help: string;
    choice: ColorChoice;
    theme: Theme;
    open: boolean;
    ontoggle: () => void;
    onchange: (c: ColorChoice) => void;
  } = $props();

  /** Typing in the custom field, before the colour is usable (not saved yet). */
  let trying = $state(false);
  let text = $state("");
  // Follow the saved value (e.g. after a reset) unless the user is typing.
  $effect.pre(() => {
    if (!trying) text = choice.custom;
  });
  const customOn = $derived(choice.customOn || trying);
  let panel = $state<HTMLElement>();
  const uid = $derived(`col-${key}`);

  // Preset colours come from the tokens of the current theme.
  const presets = $derived(
    PRESETS[key].map(([id, name]) => {
      theme; // re-read when the theme flips
      const hex = token(`--preset-${key}-${id}`).toUpperCase();
      return { id, name, hex, ink: luminance(hex) > 0.3 ? "var(--ink-on-light)" : "var(--ink-on-dark)" };
    }),
  );
  const hull = $derived(hullHex(theme));
  const preset = $derived(presets.find((p) => p.id === choice.preset) ?? presets[0]);
  const valid = $derived(isHex(text));
  const ratio = $derived(valid ? contrast(text.trim(), hull) : 0);
  const applied = $derived(choice.customOn && valid && ratio >= MIN_CONTRAST);
  const shownHex = $derived(applied ? text.trim().toUpperCase() : preset.hex);
  const shownName = $derived(applied ? "Custom" : preset.name);
  const shownRatio = $derived(customOn && valid ? ratio : contrast(preset.hex, hull));
  const error = $derived.by(() => {
    if (!customOn || !text.trim()) return "";
    if (!valid) return `Enter # and six hex digits, for example ${presets[0].hex}.`;
    if (ratio < MIN_CONTRAST)
      return `Not applied: the contrast is ${ratio.toFixed(1)}:1 on the island and must be at least ${MIN_CONTRAST}:1. Choose a ${theme === "dark" ? "lighter" : "darker"} shade.`;
    return "";
  });

  function typed(v: string) {
    text = v;
    trying = true;
    const t = v.trim();
    // Only a usable colour is saved; anything else stays local with an error.
    if (isHex(t) && contrast(t, hull) >= MIN_CONTRAST) onchange({ preset: choice.preset, custom: t, customOn: true });
  }

  // The panel slides open with GSAP (fade only with Reduce motion).
  $effect(() => {
    if (!open) return;
    tick().then(() => {
      if (!panel) return;
      gsap.fromTo(
        panel,
        { height: reduced() ? "auto" : 0, autoAlpha: 0 },
        { height: "auto", autoAlpha: 1, duration: dur("fast"), ease: ease(), clearProps: "height" },
      );
    });
  });
</script>

<Row {label} {help} note={error} noteIcon="alert" noteTone="err">
  <button
    class="trigger"
    aria-expanded={open}
    aria-controls="{uid}-panel"
    aria-label="{label} colour: {shownName}, {shownHex}. {open ? 'Close' : 'Open'} colour choices."
    onclick={ontoggle}
  >
    <span class="swatch" style:background={applied ? text.trim() : `var(--preset-${key}-${preset.id})`}></span>
    <span class="name">{shownName}</span>
    <span class="hex tabular">{shownHex}</span>
    <Icon name={open ? "chevronUp" : "chevronDown"} size={16} />
  </button>
  {#snippet below()}
    {#if open}
      <div class="panel" id="{uid}-panel" bind:this={panel}>
        <div class="presets" role="radiogroup" aria-label="{label} colour presets">
          {#each presets as p (p.id)}
            {@const on = !applied && p.id === choice.preset}
            <button
              class="preset"
              class:on
              role="radio"
              aria-checked={on}
              onclick={() => {
                trying = false;
                onchange({ preset: p.id, custom: choice.custom, customOn: false });
              }}
            >
              <span class="dot" style:background="var(--preset-{key}-{p.id})" style:color={p.ink}>
                {#if on}<Icon name="check" size={14} />{/if}
              </span>
              <span>{p.name}</span>
            </button>
          {/each}
        </div>
        <div class="custom">
          <label for="{uid}-hex">Custom</label>
          <input
            id="{uid}-hex"
            class="d-field hexfield"
            type="text"
            spellcheck="false"
            maxlength="7"
            placeholder="#RRGGBB"
            value={text}
            aria-invalid={!!error}
            oninput={(e) => typed(e.currentTarget.value)}
          />
          <span class="ratio" class:bad={shownRatio < MIN_CONTRAST}>
            <Icon name={shownRatio >= MIN_CONTRAST ? "check" : "alert"} size={16} />
            <span>{customOn && valid ? "Custom" : preset.name} has {shownRatio.toFixed(1)}:1 contrast on the island</span>
          </span>
        </div>
      </div>
    {/if}
  {/snippet}
</Row>

<style>
  .trigger {
    flex: none;
    box-sizing: border-box;
    height: var(--size-control);
    padding: 0 var(--sp-10) 0 var(--sp-6);
    border-radius: var(--r-control);
    border: 1px solid var(--c-ctrl);
    background: var(--c-field);
    font-size: var(--fs-small);
    display: flex;
    align-items: center;
    gap: var(--sp-8);
  }
  .trigger:hover {
    background-image: linear-gradient(var(--c-hover), var(--c-hover));
  }
  .swatch {
    width: var(--size-swatch);
    height: var(--size-swatch);
    border-radius: var(--r-nav);
    box-shadow: inset 0 0 0 1px var(--c-line);
  }
  .name {
    min-width: var(--size-number);
    text-align: left;
  }
  .hex {
    color: var(--c-muted);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--sp-12);
    padding: var(--sp-12);
    border-radius: var(--r-control);
    background: var(--c-win);
    box-shadow: inset 0 0 0 1px var(--c-line);
    overflow: hidden;
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-8);
  }
  .preset {
    box-sizing: border-box;
    height: var(--size-button);
    padding: 0 var(--sp-12) 0 var(--sp-8);
    border-radius: var(--r-control);
    border: 1px solid var(--c-ctrl);
    background: var(--c-card);
    font-size: var(--fs-small);
    display: flex;
    align-items: center;
    gap: var(--sp-8);
  }
  .preset.on {
    border: var(--focus-width) solid var(--c-ink);
  }
  .dot {
    width: var(--size-preset-dot);
    height: var(--size-preset-dot);
    border-radius: var(--r-pill);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .custom {
    display: flex;
    align-items: center;
    gap: var(--sp-12);
    flex-wrap: wrap;
  }
  label {
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .hexfield {
    width: var(--size-hex-field);
    padding: 0 var(--sp-10);
  }
  .ratio {
    display: flex;
    align-items: center;
    gap: var(--sp-6);
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .ratio.bad {
    color: var(--c-err);
  }
</style>
