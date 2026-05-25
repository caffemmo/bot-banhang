    async function loadAdmins() {
      try {
        const users = await apiFetch('/users');
        const rows = users.map(u => `
        <tr>
          <td>${u.id}</td>
          <td class="fw-semibold">${escapeHtml(u.username)}</td>
          <td class="small text-muted">${escapeHtml(u.last_login_at || '-')}</td>
          <td>
            <button class="btn btn-sm btn-outline-primary change-admin-password" data-id="${u.id}" data-username="${escapeAttr(u.username)}">Đổi mật khẩu</button>
          </td>
        </tr>
      `).join('');
        $('#admins-body').html(rows || '<tr><td colspan="4" class="text-center text-muted">Chưa có admin</td></tr>');
      } catch (e) {
        alertBox('danger', 'Tải danh sách admin thất bại: ' + e.message);
      }
    }

    async function createAdminUser(e) {
      e.preventDefault();
      const payload = {
        username: $('#admin-create-username').val().trim(),
        password: $('#admin-create-password').val(),
        setup_code: $('#admin-create-code').val().trim(),
      };
      try {
        await apiFetch('/users', { method: 'POST', body: JSON.stringify(payload) });
        $('#admin-create-form')[0].reset();
        alertBox('success', 'Đã tạo admin mới.');
        await loadAdmins();
      } catch (e) {
        alertBox('danger', 'Tạo admin thất bại: ' + e.message);
      }
    }

    function openChangeAdminPassword(id, username) {
      showInputActionModal({
        title: 'Đổi mật khẩu admin',
        description: `Tài khoản: ${username}`,
        confirmText: 'Đổi mật khẩu',
        confirmClass: 'btn-primary',
        fields: [
          { name: 'password', label: 'Mật khẩu mới', type: 'password', required: true },
          { name: 'setup_code', label: 'Mã bí mật', type: 'password', required: true },
        ],
        onSubmit: async (values) => {
          await apiFetch(`/users/${id}/password`, {
            method: 'PUT',
            body: JSON.stringify({
              password: values.password,
              setup_code: values.setup_code,
            }),
          });
          alertBox('success', 'Đã đổi mật khẩu admin.');
          await loadAdmins();
        },
      });
    }

