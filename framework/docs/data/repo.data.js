const API_URL = "https://api.github.com/repos/BumpyClock/gpui-component";

export default {
  async load() {
    return await fetch(API_URL).then((res) => res.json());
  },
};
