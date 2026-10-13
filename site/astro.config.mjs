// @ts-check
import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import preact from '@astrojs/preact';
import starlightLinksValidator from 'starlight-links-validator';
import YAML from 'yaml';

// The Reference sidebar comes from the page map, so a published repository file is listed in
// exactly one place.
const map = YAML.parse(readFileSync(new URL('./docs-map.yaml', import.meta.url), 'utf8'));
const reference = map.sections.map((section) => ({
  label: section,
  collapsed: true,
  items: [
    ...(section === 'Configuration'
      ? [{ label: 'Configuration file', link: '/reference/configuration/config-file/' }]
      : []),
    ...map.pages
      .filter((p) => p.section === section)
      .map((p) => (p.label ? { label: p.label, slug: p.slug } : p.slug)),
  ],
}));

export default defineConfig({
  site: 'https://thin-edge.github.io',
  base: '/tedge-dot',
  trailingSlash: 'ignore',
  // The tools import the contract schemas and test vectors from the repository (../doc).
  vite: { server: { fs: { allow: ['..'] } } },
  integrations: [
    starlight({
      title: 'tedge-dot',
      description:
        'OT protocol connectors for thin-edge.io: Modbus, OPC UA, CAN bus, CANopen, SNMP and PROFIBUS-DP into one neutral MQTT contract.',
      logo: { src: './src/assets/mark.svg', alt: '' },
      favicon: '/favicon.svg',
      customCss: [
        '@fontsource-variable/inter',
        '@fontsource-variable/jetbrains-mono',
        './src/styles/theme.css',
      ],
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/thin-edge/tedge-dot' }],
      editLink: { baseUrl: 'https://github.com/thin-edge/tedge-dot/edit/main/site/' },
      lastUpdated: false,
      // Robot Framework has no Shiki grammar; show those blocks as plain text.
      expressiveCode: { shiki: { langAlias: { robotframework: 'txt', robot: 'txt' } } },
      components: {
        // The landing page's animated data-flow hero.
        Hero: './src/components/LandingHero.astro',
      },
      head: [
        { tag: 'meta', attrs: { name: 'theme-color', content: '#0b1f33' } },
      ],
      sidebar: [
        {
          label: 'Get started',
          items: [
            { label: 'Overview', slug: 'get-started/overview' },
            { label: 'Install', slug: 'get-started/install' },
            { label: 'Your first device', slug: 'get-started/first-device' },
          ],
        },
        { label: 'Guides', items: [{ autogenerate: { directory: 'guides' } }] },
        {
          label: 'Tools',
          items: [
            { label: 'Config builder', slug: 'tools/config-builder' },
            { label: 'Package picker', slug: 'tools/package-picker' },
            { label: 'Report-policy playground', slug: 'tools/report-playground' },
          ],
        },
        { label: 'Reference', items: reference },
        { label: "What's new", link: '/whats-new/' },
      ],
      plugins: [starlightLinksValidator({ errorOnLocalLinks: false })],
    }),
    preact(),
  ],
});
