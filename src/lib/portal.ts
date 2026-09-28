/**
 * Svelte action：把节点挂到 document.body。
 *
 * 用途：`position: fixed` 的浮层（取色框/弹出菜单）若处于带有
 * transform / filter / contain / will-change 的祖先子树内，fixed 会退化为
 * 相对该祖先定位、并可能被 overflow 裁剪 —— 导致浮层「位置错乱」。
 * 挂到 body 后，fixed 恢复为相对视口定位，left/top 用视口坐标即可精确落位。
 */
export function portal(node: HTMLElement) {
  document.body.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
