<!--
  A number that counts to its new value (original effect, in the spirit of
  react-bits' CountUp). GSAP tweens it; with Reduce motion it just changes.
-->
<script lang="ts">
  import { dur, ease, gsap, reduced } from "../../lib/motion";

  let { value, suffix = "" }: { value: number; suffix?: string } = $props();

  const box = { n: 0 };
  let shown = $state(0);
  let first = true;

  $effect(() => {
    const to = value;
    if (first || reduced()) {
      first = false;
      gsap.killTweensOf(box);
      box.n = to;
      shown = to;
      return;
    }
    gsap.to(box, { n: to, duration: dur("slow"), ease: ease(), overwrite: true, onUpdate: () => (shown = Math.round(box.n)) });
  });
</script>

<span class="tabular">{shown}{suffix}</span>
