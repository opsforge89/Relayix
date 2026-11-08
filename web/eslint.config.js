import js from '@eslint/js';
import globals from 'globals';
export default [
 {ignores:['dist/**','node_modules/**']},
 js.configs.recommended,
 {files:['**/*.{js,jsx}'],languageOptions:{ecmaVersion:'latest',sourceType:'module',parserOptions:{ecmaFeatures:{jsx:true}},globals:{...globals.browser,...globals.node,__APP_VERSION__:'readonly'}},rules:{'no-unused-vars':'off','no-empty':['error',{allowEmptyCatch:true}],'no-control-regex':'off'}},
];
