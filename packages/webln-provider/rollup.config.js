import typescript from '@rollup/plugin-typescript';
import terser from '@rollup/plugin-terser';

export default [
  // UMD bundle for injection into WebViews
  {
    input: 'src/index.ts',
    output: {
      file: 'dist/webln-provider.min.js',
      format: 'iife',
      name: '_BreezSparkWebLnModule',
      sourcemap: false,
    },
    plugins: [
      typescript({ tsconfig: './tsconfig.json' }),
      terser({
        compress: {
          drop_console: true,
        },
      }),
    ],
  },
  // ESM bundle for development/testing
  {
    input: 'src/index.ts',
    output: {
      file: 'dist/webln-provider.esm.js',
      format: 'esm',
      sourcemap: true,
    },
    plugins: [typescript({ tsconfig: './tsconfig.json' })],
  },
  // CJS bundle
  {
    input: 'src/index.ts',
    output: {
      file: 'dist/webln-provider.js',
      format: 'cjs',
      sourcemap: true,
    },
    plugins: [typescript({ tsconfig: './tsconfig.json' })],
  },
];
