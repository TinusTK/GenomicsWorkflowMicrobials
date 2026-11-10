// A small hash router: #/runs/abc -> { page: 'runs', id: 'abc' }.

function parse(hash: string) {
  const [path, query = ''] = hash.replace(/^#\/?/, '').split('?');
  const [page = 'home', id = '', ...rest] = path.split('/').filter(Boolean).map(decodeURIComponent);
  return { page, id, rest, query: new URLSearchParams(query) };
}

class Route {
  current = $state(parse(location.hash));
  constructor() {
    window.addEventListener('hashchange', () => (this.current = parse(location.hash)));
  }
}

export const route = new Route();

export function go(path: string) {
  location.hash = '#/' + path.replace(/^\//, '');
}

export function href(path: string) {
  return '#/' + path.replace(/^\//, '');
}
