// ESLint rules for the map: strict typed rules, no `any`, no `as` casts, no non-null assertions
// outside tests, the React hooks rules, no raw HTML injection, and no raw colour or size in a
// component: colours, lengths and Tailwind's numbered utilities come from the tokens only.
import js from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import tseslint from 'typescript-eslint';

/** A colour written out: `#rgb` to `#rrggbbaa`, or a CSS colour function. */
const RAW_COLOUR = String.raw`#[0-9a-fA-F]{3,8}\b|\b(rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(`;

/** A length written out in pixels or font units. */
const RAW_LENGTH = String.raw`\d(px|rem|em|pt)\b`;

/** A Tailwind utility sized by a number of its default scale (`p-4`, `h-8`, `text-2xl`); 0 is no size. */
const NUMBERED_UTILITY = String.raw`(^|\s|:)-?(p[xytrbl]?|m[xytrbl]?|gap(-[xy])?|[wh]|size|(min|max)-[wh]|top|left|right|bottom|inset(-[xy])?|space-[xy]|leading|tracking|rounded(-[a-z]+)?)-(0\.|[1-9])|(^|\s|:)text-(xs|sm|base|lg|[0-9]?xl)\b`;

/** SVG and HTML attributes that take a size. */
const SIZE_ATTRIBUTES =
  '/^(width|height|x|y|cx|cy|r|rx|ry|x1|x2|y1|y2|strokeWidth|fontSize|viewBox)$/';

/** One restriction per pattern, on string literals and template text alike. */
function rawValueRules(pattern, message) {
  return [
    { selector: `Literal[value=/${pattern}/]`, message },
    { selector: `TemplateElement[value.raw=/${pattern}/]`, message },
  ];
}

const RAW_VALUE_RULES = [
  ...rawValueRules(RAW_COLOUR, 'Write colours in src/theme/tokens.ts and read them from there.'),
  ...rawValueRules(RAW_LENGTH, 'Write sizes in src/theme/tokens.ts and read them from there.'),
  ...rawValueRules(NUMBERED_UTILITY, "Use the tokens' utilities (p-sm, h-control), not numbers."),
  {
    selector: `JSXAttribute[name.name=${SIZE_ATTRIBUTES}] > Literal`,
    message: 'A size attribute takes its value from the tokens.',
  },
  {
    selector: `JSXAttribute[name.name=${SIZE_ATTRIBUTES}] > JSXExpressionContainer > Literal`,
    message: 'A size attribute takes its value from the tokens.',
  },
  {
    selector: "JSXAttribute[name.name='style'] Property > Literal",
    message: 'An inline style takes its values from the tokens.',
  },
];

export default tseslint.config(
  { ignores: ['dist/', 'node_modules/', 'coverage/', 'playwright-report/', 'test-results/'] },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  reactHooks.configs.flat.recommended,
  {
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-non-null-assertion': 'error',
      '@typescript-eslint/consistent-type-assertions': ['error', { assertionStyle: 'never' }],
      '@typescript-eslint/consistent-type-imports': 'error',
      '@typescript-eslint/restrict-template-expressions': ['error', { allowNumber: true }],
      'no-restricted-properties': [
        'error',
        { property: 'innerHTML', message: 'Render untrusted text as text, never as HTML.' },
        { property: 'outerHTML', message: 'Render untrusted text as text, never as HTML.' },
        { property: 'insertAdjacentHTML', message: 'Render untrusted text as text.' },
      ],
      'no-restricted-syntax': [
        'error',
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: 'Render untrusted text as text, never as HTML.',
        },
      ],
    },
  },
  {
    // Components hold no raw colour or size; the tokens module is where those are written.
    files: ['src/**/*.tsx'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: 'Render untrusted text as text, never as HTML.',
        },
        ...RAW_VALUE_RULES,
      ],
    },
  },
  {
    files: ['**/*.test.ts'],
    rules: {
      '@typescript-eslint/consistent-type-assertions': 'off',
      '@typescript-eslint/no-non-null-assertion': 'off',
    },
  },
  {
    files: ['**/*.js'],
    ...tseslint.configs.disableTypeChecked,
  },
);
