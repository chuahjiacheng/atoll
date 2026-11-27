<script lang="ts">
  import Pip, { type Mood, type Tone } from "../../lib/ui/Pip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { px } from "../../lib/theme";
  import { NAP_CHOICES, OUTFITS } from "../../lib/pip";
  import type { Settings } from "../../lib/settings";
  import { change } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SegRow from "../ui/SegRow.svelte";
  import Seg from "../ui/Seg.svelte";
  import Switch from "../ui/Switch.svelte";

  let { s }: { s: Settings } = $props();

  // Preview only: not saved.
  let mood = $state<Mood>("happy");
  let music = $state(false);
  const TONES: { id: Tone; name: string }[] = [
    { id: "peri", name: "Periwinkle" },
    { id: "mint", name: "Mint" },
    { id: "peach", name: "Peach" },
    { id: "lilac", name: "Lilac" },
  ];
  const big = px("--size-pip-xl");
  const small = px("--size-pip-outfit");
  const tiny = px("--size-pip-peek");

  const napText = $derived(s.mascot.napAfter ? `after ${s.mascot.napAfter} quiet minutes` : "never (napping is off)");
  /** What makes Pip show each mood in the island. */
  const MEANINGS = $derived<{ mood: Mood; name: string; when: string }[]>([
    { mood: "idle", name: "Idle", when: "Nothing is happening. Pip waits next to a quiet dot." },
    { mood: "focused", name: "Focused", when: "Claude is working, or Ask is thinking about an answer." },
    { mood: "curious", name: "Curious", when: "Claude needs you: a permission or a question." },
    { mood: "happy", name: "Happy", when: s.mascot.celebrate ? "Claude finished. Pip hops to tell you." : "Claude finished." },
    { mood: "dizzy", name: "Dizzy", when: "Something went wrong, or you poked Pip a few times in a row." },
    { mood: "sleepy", name: "Napping", when: `The island has been quiet: ${napText}. Anything new wakes Pip up.` },
  ]);
</script>

<Section title="Preview">
  <Row line={false}>
    {#snippet below()}
      <div class="mp">
        <div class="stage">
          <Pip {mood} tone={s.mascot.tone} size={big} pokeable={s.mascot.reactPokes} bob={music && s.mascot.reactMusic} {music} />
        </div>
        <div class="txt">
          <span>Preview mood</span>
          <span class="h">Shows how Pip looks in each mood and outfit. Click Pip to poke it. This is a preview only and is not saved.</span>
          <label class="music">
            <Switch checked={music} label="Pretend music is playing" onchange={(v) => (music = v)} />
            <span>Pretend music is playing</span>
          </label>
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
          { id: "sleepy", label: "Napping" },
        ]}
        value={mood}
        fit
        onchange={(v) => (mood = v as Mood)}
      />
    {/snippet}
  </Row>
</Section>

<Section title="Look">
  <Row label="Outfit" help="What Pip wears everywhere in the island and in Settings.">
    {#snippet below()}
      <div class="outfits" role="radiogroup" aria-label="Outfit">
        {#each OUTFITS as o (o.id)}
          {@const on = o.id === s.mascot.outfit}
          <button class="outfit" class:on role="radio" aria-checked={on} onclick={() => change("mascot.outfit", o.id)}>
            <Pip mood="idle" tone={s.mascot.tone} size={small} outfit={o.id} music={false} still />
            <span>{o.name}</span>
          </button>
        {/each}
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
    label="Headphones with music"
    help="Pip swaps its outfit for headphones while music is playing."
    checked={s.mascot.headphonesWithMusic}
    disabled={!s.modules.mascot}
    onchange={(v) => change("mascot.headphonesWithMusic", v)}
  />
  <SwitchRow
    label="Celebrate finished work"
    help="Pip hops when Claude finishes a task."
    checked={s.mascot.celebrate}
    disabled={!s.modules.mascot}
    onchange={(v) => change("mascot.celebrate", v)}
  />
  <SwitchRow
    label="Blink"
    help="Pip blinks every few seconds when its eyes are open. Off when Windows asks for less motion."
    checked={s.mascot.blink}
    disabled={!s.modules.mascot}
    onchange={(v) => change("mascot.blink", v)}
  />
  <SegRow
    label="Nap when quiet"
    help="Pip dozes off in the collapsed island when nothing has happened for a while."
    items={NAP_CHOICES.map((m) => ({ id: String(m), label: m ? `${m} min` : "Never" }))}
    value={String(s.mascot.napAfter)}
    disabled={!s.modules.mascot}
    onchange={(v) => change("mascot.napAfter", Number(v))}
  />
  <SwitchRow
    label="React to pokes"
    help="Click Pip a few times and it gets dizzy."
    checked={s.mascot.reactPokes}
    disabled={!s.modules.mascot}
    onchange={(v) => change("mascot.reactPokes", v)}
  />
</Section>

<Section title="What Pip's moods mean">
  <Row line={false}>
    {#snippet below()}
      <ul class="moods">
        {#each MEANINGS as m (m.mood)}
          <li>
            <span class="mbox"><Pip mood={m.mood} tone={s.mascot.tone} size={tiny} still /></span>
            <span class="mtxt">
              <span class="mname">{m.name}</span>
              <span class="h">{m.when}</span>
            </span>
          </li>
        {/each}
      </ul>
    {/snippet}
  </Row>
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
  .music {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    margin-top: var(--sp-8);
    font-size: var(--fs-small);
    cursor: pointer;
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
    padding: var(--sp-8);
    border-radius: var(--r-card);
    font-size: var(--fs-small);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-8);
    text-align: center;
    box-shadow: inset 0 0 0 1px var(--c-line);
  }
  .outfit:hover {
    background: var(--c-desk);
  }
  .outfit.on {
    background: var(--c-desk);
    box-shadow: 0 0 0 var(--focus-width) var(--c-accent);
    font-weight: var(--fw-strong);
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
  .moods {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--size-drop), 1fr));
    gap: var(--sp-12) var(--sp-24);
  }
  .moods li {
    display: flex;
    align-items: center;
    gap: var(--sp-12);
  }
  .mbox {
    width: var(--size-control);
    height: var(--size-control);
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .mtxt {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2, 2px);
  }
  .mname {
    font-weight: var(--fw-strong);
  }
</style>
