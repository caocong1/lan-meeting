/* @refresh reload */
import { render } from "solid-js/web";
import "@unocss/reset/tailwind.css";
import "virtual:uno.css";
import { Viewer } from "./components/Viewer";

const root = document.getElementById("root");

if (root) {
  render(() => <Viewer />, root);
}
