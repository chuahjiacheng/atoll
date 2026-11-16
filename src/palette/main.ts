// The command palette window (search-clipboard branch). See src-tauri/src/search/palette.rs.
import { mount } from "svelte";
import "../app.css";
import Palette from "../lib/palette/Palette.svelte";

mount(Palette, { target: document.getElementById("app")! });
