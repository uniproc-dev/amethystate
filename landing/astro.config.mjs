// @ts-check

import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'astro/config';

import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
  site: 'https://uniproc-dev.github.io',
  base: '/amethystate',

  vite: {
      plugins: [tailwindcss()],
	},

  integrations: [starlight({
      title: 'amethystate',
      customCss: ['./src/styles/starlight.css'],
      routeMiddleware: './src/starlightRouteData.ts',

      defaultLocale: 'root',
      locales: {
          root: { label: 'English', lang: 'en' },
          ru: { label: 'Русский', lang: 'ru' },
      },

      sidebar: [
          { slug: 'introduction' },
          { slug: 'architecture' },
          {
              label: 'Getting started',
              translations: { ru: 'Начало работы' },
              items: [{ autogenerate: { directory: 'Getting-started' } }],
          },
          {
              label: 'State',
              translations: { ru: 'Состояние' },
              items: [{ autogenerate: { directory: 'State' } }],
          },
          {
              label: 'Primitives',
              translations: { ru: 'Примитивы' },
              items: [{ autogenerate: { directory: 'Primitives' } }],
          },
          {
              label: 'Store',
              translations: { ru: 'Store' },
              items: [{ autogenerate: { directory: 'Store' } }],
          },
          {
              label: 'Concepts',
              translations: { ru: 'Концепты' },
              items: [{ autogenerate: { directory: 'Concepts' } }],
          },
          {
              label: 'Migrations',
              translations: { ru: 'Миграции' },
              items: [{ autogenerate: { directory: 'Migrations' } }],
          },
          {
              label: 'Limitations',
              translations: { ru: 'Ограничения' },
              items: [{ autogenerate: { directory: 'Limitations' } }],
          },
          {
              label: 'Integrations',
              translations: { ru: 'Интеграции' },
              items: [{ autogenerate: { directory: 'Integrations' } }],
          },
      ],
  })],
});