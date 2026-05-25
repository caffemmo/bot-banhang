    $(document).ready(function () {
      $('button').each(function () {
        const btn = $(this);
        const label = (btn.text() || '').trim();
        if (!btn.attr('aria-label') && (label === '↻' || label.length === 1)) {
          btn.attr('aria-label', 'Nút thao tác');
        }
      });
      bindEvents();
      checkAuth();
    });
