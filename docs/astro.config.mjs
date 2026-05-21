// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

// https://astro.build/config
export default defineConfig({
  site: "https://mkorbi.github.io",
  base: "/ravn",
  integrations: [
    starlight({
      title: "ravn",
      description:
        "A personal-assistant AI agent in Rust — ReAct loop, MCP, skills, hybrid search.",
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/mkorbi/ravn",
        },
      ],
      editLink: {
        baseUrl: "https://github.com/mkorbi/ravn/edit/main/docs/",
      },
      sidebar: [
        {
          label: "Getting Started",
          autogenerate: { directory: "getting-started" },
        },
        {
          label: "User Guide",
          autogenerate: { directory: "user-guide" },
        },
        {
          label: "Architecture",
          autogenerate: { directory: "architecture" },
        },
      ],
    }),
  ],
});
