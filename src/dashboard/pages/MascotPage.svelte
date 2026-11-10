<script lang="ts">
  import Pip, { type Mood, type Tone } from "../../lib/ui/Pip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { px } from "../../lib/theme";
  import type { Settings } from "../../lib/settings";
  import { change } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import Seg from "../ui/Seg.svelte";

  let { s }: { s: Settings } = $props();

  // Preview only: not saved.
  let mood = $state<Mood>("happy");
  const TONES: { id: Tone; name: string }[] = [
    { id: "peri", name: "Periwinkle" },
    { id: "mint", name: "Mint" },
    { id: "peach", name: "Peach" },
    { id: "lilac", name: "Lilac" },
  ];
  const big = px("--size-pip-xl");
  const small = px("--size-pip-outfit");
</script>

<Section title="Preview">
  <Row line={false}>
    {#snippet below()}
      <div class="mp">
        <div class="stage"><Pip {mood} tone={s.mascot.tone} size={big} pokeable={s.mascot.reactPokes} /></div>
        <div class="txt">
          <span>Preview mood</span>
          <span class="h">Shows how Pip looks in each mood. This is a preview only and is not saved.</span>
        </div>
      </div>
      <Seg
        label="Preview mood"
        items={[
          { id: "idle", label: "Idle" },
          { id: "focused", label: "Focused" },
          { id: "curious", label: "Curious" },
          { id: "happy", label: "Happy" },
          { id: "dizzy", label: "Dizzy" },
        ]}
        value={mood}
        fit
        onchange={(v) => (mood = v as Mood)}
      />
    {/snippet}
  </Row>
</Section>

<Section title="Look">
  <Row label="Outfit" help="Only the default look exists so far.">
    {#snippet below()}
      <div class="outfits" role="radiogroup" aria-label="Outfit">
        <button class="outfit on" role="radio" aria-checked="true">
          <Pip mood="idle" tone={s.mascot.tone} size={small} />
          <span>None</span>
        </button>
        <button class="outfit todo" role="radio" aria-checked="false" disabled>Outfit to be designed</button>
        <button class="outfit todo" role="radio" aria-checked="false" disabled>Outfit to be designed</button>
      </div>
    {/snippet}
  </Row>
  <Row label="Colour" help="Pip's body colour.">
    <div class="swatches" role="radiogroup" aria-label="Colour">
      {#each TONES as t (t.id)}
        {@const on = t.id === s.mascot.tone}
        <button
          class="sw"
          class:on
          role="radio"
          aria-checked={on}
          aria-label={t.name}
          style:background="var(--pip-{t.id})"
          onclick={() => change("mascot.tone", t.id)}
        >
          {#if on}<Icon name="check" size={18} />{/if}
        </button>
      {/each}
    </div>
  </Row>
</Section>

<Section title="Reactions">
  <SwitchRow label="Show Pip" help="Pip in the island header and the collapsed island." checked={s.modules.mascot} onchange={(v) => change("modules.mascot", v)} />
  <SwitchRow
    label="React to music"
    help="Pip bobs along while music is playing."
    checked={s.mascot.reactMusic}
    disabled={!s.modules.mascot}
    onchange={(v) => change("mascot.reactMusic", v)}
  />
  <SwitchRow
    label="React to pokes"
    help="Click Pip a few times and it gets dizzy."
    checked={s.mascot.reactPokes}
    disabled={!s.modules.mascot}
    onchange={(v) => change("mascot.reactPokes", v)}
  />
</Section>

<style>
  .mp {
    display: flex;
    gap: var(--sp-24);
    align-items: center;
    flex-wrap: wrap;
  }
  .stage {
    width: var(--size-mood-box);
    height: var(--size-mood-box);
    flex: none;
    border-radius: var(--r-card);
    background: var(--c-desk);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .txt {
    flex: 1;
    min-width: var(--size-drop);
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }
  .h {
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .outfits {
    display: flex;
    gap: var(--sp-12);
    flex-wrap: wrap;
  }
  .outfit {
    box-sizing: border-box;
    width: var(--size-outfit-w);
    height: var(--size-outfit-h);
    padding: var(--sp-12);
    border-radius: var(--r-card);
    font-size: var(--fs-small);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-8);
    text-align: center;
  }
  .outfit.on {
    padding: var(--sp-8);
    background: var(--c-desk);
    box-shadow: 0 0 0 var(--focus-width) var(--c-accent);
    font-weight: var(--fw-strong);
  }
  .outfit.todo {
    border: var(--dashed) dashed var(--c-ctrl);
    color: var(--c-muted);
  }
  .swatches {
    display: flex;
    gap: var(--sp-8);
  }
  .sw {
    width: var(--size-control);
    height: var(--size-control);
    border-radius: var(--r-pill);
    box-shadow: inset 0 0 0 1px var(--c-line);
    color: var(--pip-face);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .sw.on {
    box-shadow:
      0 0 0 var(--focus-width) var(--c-card),
      0 0 0 calc(var(--focus-width) * 2) var(--c-ink);
  }
</style>
