// Expo's preset for Metro and for jest (jest-expo's babel-jest reads this file).
module.exports = function config(api) {
  api.cache(true);
  return { presets: ["babel-preset-expo"] };
};
